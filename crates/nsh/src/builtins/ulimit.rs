//! The `ulimit` builtin.

use bstr::BStr;
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

// [spec:posix:req:builtin.ulimit.opt-core]
// [spec:posix:req:builtin.ulimit.opt-data]
// [spec:posix:req:builtin.ulimit.opt-fsize]
// [spec:posix:req:builtin.ulimit.opt-nofile]
// [spec:posix:req:builtin.ulimit.opt-stack]
// [spec:posix:req:builtin.ulimit.opt-cpu]
// [spec:posix:req:builtin.ulimit.opt-as]
static LIMITS: [Limit; 12] = [
    Limit {
        name: b"CPU time (seconds)",
        resource: LimitResource::Cpu,
        factor: 1,
        option: b't',
    },
    Limit {
        name: b"file size (512-byte units)",
        resource: LimitResource::FileSize,
        factor: 512,
        option: b'f',
    },
    Limit {
        name: b"data segment size (1024-byte units)",
        resource: LimitResource::Data,
        factor: 1024,
        option: b'd',
    },
    Limit {
        name: b"stack size (1024-byte units)",
        resource: LimitResource::Stack,
        factor: 1024,
        option: b's',
    },
    Limit {
        name: b"core file size (512-byte units)",
        resource: LimitResource::Core,
        factor: 512,
        option: b'c',
    },
    Limit {
        name: b"resident memory (1024-byte units)",
        resource: LimitResource::ResidentSet,
        factor: 1024,
        option: b'm',
    },
    Limit {
        name: b"locked memory (1024-byte units)",
        resource: LimitResource::LockedMemory,
        factor: 1024,
        option: b'l',
    },
    Limit {
        name: b"processes",
        resource: LimitResource::Processes,
        factor: 1,
        option: b'p',
    },
    Limit {
        name: b"open files",
        resource: LimitResource::OpenFiles,
        factor: 1,
        option: b'n',
    },
    Limit {
        name: b"address space (1024-byte units)",
        resource: LimitResource::AddressSpace,
        factor: 1024,
        option: b'v',
    },
    Limit {
        name: b"file locks",
        resource: LimitResource::Locks,
        factor: 1,
        option: b'w',
    },
    Limit {
        name: b"realtime priority",
        resource: LimitResource::RealtimePriority,
        factor: 1,
        option: b'r',
    },
];

// [spec:dash:def:miscbltin.limtype]
#[derive(Clone, Copy)]
struct LimitSelection {
    current: bool,
    maximum: bool,
}

impl LimitSelection {
    const BOTH: Self = Self {
        current: true,
        maximum: true,
    };
    const CURRENT: Self = Self {
        current: true,
        maximum: false,
    };
    const MAXIMUM: Self = Self {
        current: false,
        maximum: true,
    };
}

// [spec:dash:def:miscbltin.printlim-fn]
// [spec:dash:sem:miscbltin.printlim-fn]
// [spec:posix:req:builtin.ulimit.stdout-single-limit-format]
fn print_limit(sh: &mut Shell, selection: LimitSelection, values: ResourceLimit, limit: Limit) {
    let value = if selection.current {
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

// [spec:posix:req:builtin.ulimit.unlimited-value]
// [spec:posix:def:builtin.ulimit.operand-newlimit]
fn parse_value(sh: &mut Shell, text: &BStr, factor: u64) -> Result<Option<u64>, Error> {
    if text == b"unlimited" {
        return Ok(None);
    }
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return Err(sh.diagnostics().sh_error_value(b"bad number"));
    }
    let value = text.iter().fold(0_u64, |value, digit| {
        value.wrapping_mul(10).wrapping_add((digit - b'0') as u64)
    });
    Ok(Some(value.wrapping_mul(factor)))
}

// [spec:dash:def:miscbltin.ulimitcmd-fn]
// [spec:dash:sem:miscbltin.ulimitcmd-fn]
// [spec:posix:syn:builtin.ulimit.syn]
// [spec:posix:req:builtin.ulimit.report-or-set]
// [spec:posix:sem:builtin.ulimit.soft-and-hard-limits]
// [spec:posix:req:builtin.ulimit.limits-exceeded]
// [spec:posix:req:builtin.ulimit.utility-syntax-guidelines]
// [spec:posix:req:builtin.ulimit.opt-hard]
// [spec:posix:req:builtin.ulimit.opt-soft]
// [spec:posix:req:builtin.ulimit.opt-all]
// [spec:posix:req:builtin.ulimit.default-hard-and-soft]
// [spec:posix:req:builtin.ulimit.default-f-option]
// [spec:posix:sem:builtin.ulimit.repeated-option-unspecified]
// [spec:posix:req:builtin.ulimit.env-locale]
// [spec:posix:req:builtin.ulimit.env-nlspath]
// [spec:posix:req:builtin.ulimit.stdout-used-when-reporting]
// [spec:posix:req:builtin.ulimit.stdout-all-format]
// [spec:posix:req:builtin.ulimit.stderr]
// [spec:posix:req:builtin.ulimit.interfaces]
// [spec:posix:req:builtin.ulimit.exit-status]
pub fn ulimitcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut selection = LimitSelection::BOTH;
    let mut all = false;
    let mut selected = b'f';
    let mut options = crate::options::Options::new(args);
    while let Some(option) = options.next(&mut sh.diagnostics(), b"HSatfdscmlpnvwr")? {
        match option {
            b'H' => selection = LimitSelection::MAXIMUM,
            b'S' => selection = LimitSelection::CURRENT,
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
        return Err(sh.diagnostics().sh_error_value(b"too many arguments"));
    }

    if all {
        for limit in LIMITS {
            let values = nsh_platform::resource_limit(limit.resource)
                .expect("a supported resource has a limit");
            let mut label = limit.name.to_vec();
            label.extend_from_slice(b" (-");
            label.push(limit.option);
            label.extend_from_slice(b") ");
            let _ = sh.io.stdout().write_all(&label);
            print_limit(sh, selection, values, limit);
        }
        return Ok(Flow::Done((0).into()));
    }

    let mut values =
        nsh_platform::resource_limit(limit.resource).expect("a supported resource has a limit");
    if let Some(argument) = operands.first() {
        let value = parse_value(sh, argument, limit.factor)?;
        if selection.maximum {
            values.maximum = value;
        }
        if selection.current {
            values.current = value;
        }
        if let Err(error) = nsh_platform::set_resource_limit(limit.resource, values) {
            let message = format!("error setting limit ({})", sh.locale.error_message(&error));
            return Err(sh.diagnostics().sh_error_value(message.as_bytes()));
        }
    } else {
        print_limit(sh, selection, values, limit);
    }
    Ok(Flow::Done((0).into()))
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
        assert!(
            LIMITS
                .iter()
                .all(|limit| !matches!(limit.option, b'H' | b'S'))
        );
    }
}
