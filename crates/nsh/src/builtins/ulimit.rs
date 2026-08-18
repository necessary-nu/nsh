//! The `ulimit` builtin.

use bstr::BStr;
use core::ffi::c_int;
use std::io::Write as _;

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use nsh_platform::{LimitResource, ResourceLimit};

// [spec:dash:def:miscbltin.limits]
#[derive(Clone, Copy)]
struct Limit {
    name: &'static [u8],
    resource: LimitResource,
    factor: u64,
    option: u8,
}

static LIMITS: [Limit; 12] = [
    Limit { name: b"time(seconds)", resource: LimitResource::Cpu, factor: 1, option: b't' },
    Limit { name: b"file(blocks)", resource: LimitResource::FileSize, factor: 512, option: b'f' },
    Limit { name: b"data(kbytes)", resource: LimitResource::Data, factor: 1024, option: b'd' },
    Limit { name: b"stack(kbytes)", resource: LimitResource::Stack, factor: 1024, option: b's' },
    Limit { name: b"coredump(blocks)", resource: LimitResource::Core, factor: 512, option: b'c' },
    Limit { name: b"memory(kbytes)", resource: LimitResource::ResidentSet, factor: 1024, option: b'm' },
    Limit { name: b"locked memory(kbytes)", resource: LimitResource::LockedMemory, factor: 1024, option: b'l' },
    Limit { name: b"process", resource: LimitResource::Processes, factor: 1, option: b'p' },
    Limit { name: b"nofiles", resource: LimitResource::OpenFiles, factor: 1, option: b'n' },
    Limit { name: b"vmemory(kbytes)", resource: LimitResource::AddressSpace, factor: 1024, option: b'v' },
    Limit { name: b"locks", resource: LimitResource::Locks, factor: 1, option: b'w' },
    Limit { name: b"rtprio", resource: LimitResource::RealtimePriority, factor: 1, option: b'r' },
];

// [spec:dash:def:miscbltin.limtype]
type LimitType = c_int;
const SOFT: LimitType = 0x1;
const HARD: LimitType = 0x2;

// [spec:dash:def:miscbltin.printlim-fn]
// [spec:dash:sem:miscbltin.printlim-fn]
fn print_limit(sh: &mut Shell, how: LimitType, values: ResourceLimit, limit: Limit) {
    let value = if how & SOFT != 0 {
        values.current
    } else {
        values.maximum
    };
    match value {
        None => {
            let _ = writeln!(sh.io.stdout(), "unlimited");
        }
        Some(value) => {
            let signed = (value / limit.factor) as i64;
            let _ = writeln!(sh.io.stdout(), "{signed}");
        }
    }
}

fn parse_value(sh: &mut Shell, text: &BStr, factor: u64) -> Result<Option<u64>, Error> {
    if text == b"unlimited" {
        return Ok(None);
    }
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return Err(sh.sh_error_value(b"bad number"));
    }
    let value = text.iter().fold(0_u64, |value, digit| {
        value.wrapping_mul(10).wrapping_add((digit - b'0') as u64)
    });
    Ok(Some(value.wrapping_mul(factor)))
}

// [spec:dash:def:miscbltin.ulimitcmd-fn]
// [spec:dash:sem:miscbltin.ulimitcmd-fn]
pub fn ulimitcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut how = SOFT | HARD;
    let mut all = false;
    let mut selected = b'f';
    let mut options = crate::options::Options::new(args);
    while let Some(option) = options.next(sh, b"HSatfdscmlpnvwr")? {
        match option {
            b'H' => how = HARD,
            b'S' => how = SOFT,
            b'a' => all = true,
            option => selected = option,
        }
    }
    let limit = *LIMITS
        .iter()
        .find(|limit| limit.option == selected)
        .expect("the option scanner and limit table agree");
    let operands = options.operands();
    if (all && !operands.is_empty()) || operands.len() > 1 {
        return Err(sh.sh_error_value(b"too many arguments"));
    }

    if all {
        for limit in LIMITS {
            let values = nsh_platform::resource_limit(limit.resource)
                .expect("a supported resource has a limit");
            let mut label = limit.name.to_vec();
            label.resize(label.len().max(20), b' ');
            label.push(b' ');
            let _ = sh.io.stdout().write_all(&label);
            print_limit(sh, how, values, limit);
        }
        return Ok(Flow::Done(0));
    }

    let mut values = nsh_platform::resource_limit(limit.resource)
        .expect("a supported resource has a limit");
    if let Some(argument) = operands.first() {
        let value = parse_value(sh, argument, limit.factor)?;
        if how & HARD != 0 {
            values.maximum = value;
        }
        if how & SOFT != 0 {
            values.current = value;
        }
        if let Err(error) = nsh_platform::set_resource_limit(limit.resource, values) {
            let message = format!(
                "error setting limit ({})",
                sh.locale.error_message(&error)
            );
            return Err(sh.sh_error_value(message.as_bytes()));
        }
    } else {
        print_limit(sh, how, values, limit);
    }
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_option_letter_has_a_row() {
        for &letter in b"tfdscmlpnvwr" {
            assert!(LIMITS.iter().any(|limit| limit.option == letter));
        }
    }

    #[test]
    fn hard_and_soft_are_not_resources() {
        assert!(LIMITS.iter().all(|limit| !matches!(limit.option, b'H' | b'S')));
    }
}
