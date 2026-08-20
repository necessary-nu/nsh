//! `umask`.
//!
//! Port of `umaskcmd` from `src/miscbltin.c`.
//!
//! "This code was ripped from pdksh 5.2.14 and hacked for use with dash
//! by Herbert Xu. Public domain."
//!
//! The mask is the process's, not the shell's: there is nothing to keep
//! here, so the builtin reads it from the kernel and writes it back.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::evaluation::Flow;
use crate::output::OutputDestination;

/*
 * umask builtin
 *
 * This code was ripped from pdksh 5.2.14 and hacked for use with
 * dash by Herbert Xu.
 *
 * Public domain.
 */

// [spec:dash:sem:miscbltin.umaskcmd-fn]
// [spec:posix:syn:builtin.umask.syn]
// [spec:posix:req:builtin.umask.set-mask]
// [spec:posix:req:builtin.umask.subshell-no-effect]
// [spec:posix:req:builtin.umask.report-when-no-operand]
// [spec:posix:req:builtin.umask.utility-syntax-guidelines]
// [spec:posix:req:builtin.umask.opt-s]
// [spec:posix:req:builtin.umask.default-output-style]
// [spec:posix:def:builtin.umask.operand-mask]
// [spec:posix:req:builtin.umask.symbolic-mode-complement]
// [spec:posix:req:builtin.umask.symbolic-op-characters]
// [spec:posix:sem:builtin.umask.non-permission-bits-unspecified]
// [spec:posix:req:builtin.umask.octal-form]
// [spec:posix:req:builtin.umask.prior-default-output-as-operand]
// [spec:posix:req:builtin.umask.env-locale]
// [spec:posix:req:builtin.umask.env-nlspath]
// [spec:posix:req:builtin.umask.stdout-no-operand]
// [spec:posix:req:builtin.umask.stdout-symbolic-format]
// [spec:posix:req:builtin.umask.stdout-operand-no-output]
// [spec:posix:req:builtin.umask.stderr]
// [spec:posix:req:builtin.umask.interfaces]
// [spec:posix:req:builtin.umask.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut mask: u32;
    let mut symbolic_mode = false;

    let mut option_scan = crate::options::Options::new(args);
    while option_scan.next(&mut shell.diagnostics(), b"S")?.is_some() {
        symbolic_mode = true;
    }
    let mode = option_scan.operands().first().copied();

    mask = crate::error::with_interrupts_deferred(shell, |_| nsh_platform::creation_mask());

    match mode {
        None => {
            if symbolic_mode {
                let allowed = !mask;
                let mut record = Vec::with_capacity(18);
                for class_index in 0..3 {
                    record.push(b"ugo"[class_index]);
                    record.push(b'=');
                    for permission_index in 0..3 {
                        if (allowed & (1 << (8 - (3 * class_index + permission_index)))) != 0 {
                            record.push(b"rwx"[permission_index]);
                        }
                    }
                    record.push(b',');
                }
                record.pop();
                record.push(b'\n');
                shell.write_output(OutputDestination::Stdout, &record)?;
            } else {
                shell.write_output_fmt(OutputDestination::Stdout, format_args!("{mask:04o}\n"))?;
            }
        }
        Some(mode) => {
            let bytes: &[u8] = mode.as_ref();
            let mut at = 0usize;
            let mut new_mask: u32;

            if bytes.first().is_some_and(u8::is_ascii_digit) {
                new_mask = 0;
                for &byte in bytes {
                    if !(b'0'..=b'7').contains(&byte) {
                        let mut message = b"Illegal number: ".to_vec();
                        message.extend_from_slice(bytes);
                        return Err(shell.diagnostics().shell_error(&message));
                    }
                    new_mask = (new_mask << 3) + u32::from(byte - b'0');
                }
            } else {
                let mut positions: u32;

                mask = !mask;
                new_mask = mask;
                positions = 0;
                let valid = 'parse: {
                    while at < bytes.len() {
                        while at < bytes.len() && b"augo".contains(&bytes[at]) {
                            match bytes[at] {
                                b'a' => positions |= 0o111,
                                b'u' => positions |= 0o100,
                                b'g' => positions |= 0o010,
                                b'o' => positions |= 0o001,
                                _ => unreachable!(),
                            }
                            at += 1;
                        }
                        if positions == 0 {
                            positions = 0o111;
                        }
                        let Some(&op) = bytes.get(at) else {
                            break 'parse false;
                        };
                        if !b"=+-".contains(&op) {
                            break 'parse false;
                        }
                        at += 1;
                        let mut permission_bits = 0u32;
                        while at < bytes.len() && b"rwxugoXs".contains(&bytes[at]) {
                            match bytes[at] {
                                b'r' => permission_bits |= 0o4,
                                b'w' => permission_bits |= 0o2,
                                b'x' => permission_bits |= 0o1,
                                b'u' => permission_bits |= mask >> 6,
                                b'g' => permission_bits |= mask >> 3,
                                b'o' => permission_bits |= mask,
                                b'X' if (mask & 0o111) != 0 => permission_bits |= 0o1,
                                b'X' | b's' => {}
                                _ => unreachable!(),
                            }
                            at += 1;
                        }
                        permission_bits = (permission_bits & 0o7) * positions;
                        match op {
                            b'-' => new_mask &= !permission_bits,
                            b'=' => new_mask = permission_bits | (new_mask & !(positions * 0o7)),
                            b'+' => new_mask |= permission_bits,
                            _ => unreachable!(),
                        }
                        match bytes.get(at).copied() {
                            Some(b',') => {
                                positions = 0;
                                at += 1;
                            }
                            Some(b'=' | b'+' | b'-') | None => {}
                            Some(_) => break 'parse false,
                        }
                    }
                    true
                };
                if !valid {
                    let mut message = b"Illegal mode: ".to_vec();
                    message.extend_from_slice(bytes);
                    return Err(shell.diagnostics().shell_error(&message));
                }
                new_mask = !new_mask;
            }
            nsh_platform::replace_creation_mask(new_mask);
        }
    }
    Ok(Flow::Done((0).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::lock;

    /// The umask is the process's, so a test has to put back what it
    /// found -- and reading it is itself a write, which is why the
    /// builtin sets zero and then restores.
    fn with_mask<T>(body: impl FnOnce() -> T) -> T {
        let _guard = lock();
        let saved = nsh_platform::creation_mask();
        let result = body();
        nsh_platform::replace_creation_mask(saved);
        result
    }

    fn set(mode: &[u8]) -> u32 {
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        assert_eq!(
            run(shell, &[BStr::new("umask"), BStr::new(mode)]).unwrap(),
            Flow::Done((0).into())
        );
        nsh_platform::creation_mask()
    }

    #[test]
    fn an_octal_operand_sets_the_mask() {
        with_mask(|| {
            assert_eq!(set(b"027"), 0o027);
            assert_eq!(set(b"0"), 0);
            /* A leading zero is not special: the number is octal either way. */
            assert_eq!(set(b"0022"), 0o022);
        });
    }

    /// The symbolic form says which bits to *allow*, so the mask it
    /// produces is their complement.
    #[test]
    fn a_symbolic_operand_is_complemented() {
        with_mask(|| {
            assert_eq!(set(b"a=rx"), 0o222);
            assert_eq!(set(b"a="), 0o777);
            assert_eq!(set(b"a=rwx"), 0);
        });
    }

    /// `+` and `-` adjust what the current mask allows rather than
    /// replacing it.
    #[test]
    fn plus_and_minus_adjust() {
        with_mask(|| {
            set(b"a=rwx");
            assert_eq!(set(b"go-w"), 0o022);
            assert_eq!(set(b"go+w"), 0);
        });
    }

    #[test]
    fn a_bad_operand_raises() {
        /* Returned rather than raised, per [dec:nsh:errors-are-values];
         * the text and the status are dash's, unchanged. */
        let _guard = lock();
        for (mode, text) in [
            ("999", &b"Illegal number: 999"[..]),
            ("q=r", &b"Illegal mode: q=r"[..]),
        ] {
            let error = run(
                &mut Shell::new(crate::streams::Streams::INHERIT),
                &[BStr::new("umask"), BStr::new(mode)],
            )
            .expect_err("a bad mode fails");
            assert_eq!(error.message().to_vec(), text.to_vec());
            assert_eq!(error.status().code(), 2);
        }
    }
}
