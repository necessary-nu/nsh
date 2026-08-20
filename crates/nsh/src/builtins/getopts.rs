//! The script-visible `getopts` scanner.
//!
//! Its persistent cursor remains in `shellparam`; each invocation scans an
//! owned snapshot of either the positional parameters or explicit operands.

use bstr::{BStr, BString};
use core::ffi::{c_int, c_uint};
use std::io::Write;

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use crate::options::Options;
use crate::var::{CallbackPolicy, VariableAttributes, set_bytes, setvarint_bytes, unset_bytes};

// [spec:dash:def:options.getoptscmd-fn]
// [spec:dash:sem:options.getoptscmd-fn]
// [spec:posix:syn:builtin.getopts.syn]
// [spec:posix:req:builtin.getopts.retrieve-options]
// [spec:posix:req:builtin.getopts.optind-initialized]
// [spec:posix:req:builtin.getopts.no-export]
// [spec:posix:sem:builtin.getopts.affects-current-environment]
// [spec:posix:def:builtin.getopts.operand-optstring]
// [spec:posix:def:builtin.getopts.operand-name]
// [spec:posix:req:builtin.getopts.operand-param]
// [spec:posix:req:builtin.getopts.env]
// [spec:posix:req:builtin.getopts.env-nlspath]
// [spec:posix:req:builtin.getopts.interfaces]
pub fn getoptscmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut opts = Options::new(args);
    opts.next(&mut sh.diagnostics(), b"")?;
    let operands = opts.operands();
    if operands.len() < 2 {
        return Err(sh
            .diagnostics()
            .sh_error_value(b"Usage: getopts optstring var [arg...]"));
    }

    let words: Vec<BString> = if operands.len() == 2 {
        let words = sh.options.shellparam.words();
        if (sh.options.shellparam.optind as c_uint) > (sh.options.shellparam.nparam + 1) as c_uint {
            sh.options.shellparam.optind = 1;
            sh.options.shellparam.optoff = -1;
        }
        words
    } else {
        if (sh.options.shellparam.optind as c_uint) > (operands.len() - 1) as c_uint {
            sh.options.shellparam.optind = 1;
            sh.options.shellparam.optoff = -1;
        }
        operands[2..]
            .iter()
            .map(|word| (*word).to_owned())
            .collect()
    };

    Ok(Flow::Done(
        (getopts(sh, operands[0], operands[1], &words)?).into(),
    ))
}

// [spec:dash:def:options.getopts-fn]
// [spec:dash:sem:options.getopts-fn]
// [spec:posix:req:builtin.getopts.optind-after-invocation]
// [spec:posix:req:builtin.getopts.unknown-option]
// [spec:posix:req:builtin.getopts.missing-option-argument]
// [spec:posix:req:builtin.getopts.end-of-options]
// [spec:posix:def:builtin.getopts.end-of-options-identification]
// [spec:posix:req:builtin.getopts.variable-set-error]
// [spec:posix:req:builtin.getopts.optstring-separate-arguments]
// [spec:posix:syn:builtin.getopts.optstring-character-restrictions]
// [spec:posix:req:builtin.getopts.optarg-content]
// [spec:posix:req:builtin.getopts.optarg]
// [spec:posix:sem:builtin.getopts.optstring-first-character]
// [spec:posix:req:builtin.getopts.stderr-diagnostic]
// [spec:posix:req:builtin.getopts.exit-status]
fn getopts(
    sh: &mut Shell,
    optstr: &BStr,
    optvar: &BStr,
    words: &[BString],
) -> Result<c_int, Error> {
    let mut option = b'?';
    let mut done = 0;
    let mut next = sh.options.shellparam.optind.saturating_sub(1) as usize;
    let offset = sh.options.shellparam.optoff;
    sh.options.shellparam.optind = -1;

    let mut cursor = if next > 0
        && offset >= 0
        && words
            .get(next - 1)
            .is_some_and(|word| word.len() >= offset as usize)
    {
        Some((next - 1, offset as usize))
    } else {
        None
    };

    'scan: loop {
        if cursor.is_none() || cursor.is_some_and(|(word, at)| at >= words[word].len()) {
            let Some(word) = words.get(next) else {
                done = 1;
                cursor = None;
                break 'scan;
            };
            if word.first() != Some(&b'-') || word.len() == 1 {
                done = 1;
                cursor = None;
                break 'scan;
            }
            next += 1;
            if word.as_slice() == b"--" {
                done = 1;
                cursor = None;
                break 'scan;
            }
            cursor = Some((next - 1, 1));
        }

        let (word_index, at) = cursor.expect("a current option word");
        option = words[word_index][at];
        cursor = Some((word_index, at + 1));

        let quiet = optstr.first() == Some(&b':');
        let mut spec = usize::from(quiet);
        while spec < optstr.len() && optstr[spec] != option {
            spec += 1;
            if optstr.get(spec) == Some(&b':') {
                spec += 1;
            }
        }
        if spec == optstr.len() {
            if quiet {
                set_bytes(
                    sh,
                    BStr::new(b"OPTARG"),
                    Some(BStr::new(&[option])),
                    VariableAttributes::NONE,
                )?;
            } else {
                let mut message = sh
                    .options
                    .arg0()
                    .unwrap_or_else(|| BStr::new(b"sh"))
                    .to_vec();
                message.extend_from_slice(b": Illegal option -");
                message.push(option);
                message.push(b'\n');
                let _ = sh.io.stderr().write_all(&message);
                unset_bytes(sh, BStr::new(b"OPTARG"))?;
            }
            option = b'?';
            break 'scan;
        }

        if optstr.get(spec + 1) == Some(&b':') {
            let (word_index, at) = cursor.expect("the option cursor");
            let argument = if at < words[word_index].len() {
                let argument = BString::from(&words[word_index][at..]);
                cursor = None;
                Some(argument)
            } else if let Some(argument) = words.get(next) {
                next += 1;
                cursor = None;
                Some(argument.clone())
            } else {
                None
            };

            let Some(argument) = argument else {
                if quiet {
                    set_bytes(
                        sh,
                        BStr::new(b"OPTARG"),
                        Some(BStr::new(&[option])),
                        VariableAttributes::NONE,
                    )?;
                    option = b':';
                } else {
                    let mut message = sh
                        .options
                        .arg0()
                        .unwrap_or_else(|| BStr::new(b"sh"))
                        .to_vec();
                    message.extend_from_slice(b": No arg for -");
                    message.push(option);
                    message.extend_from_slice(b" option\n");
                    let _ = sh.io.stderr().write_all(&message);
                    unset_bytes(sh, BStr::new(b"OPTARG"))?;
                    option = b'?';
                }
                break 'scan;
            };
            set_bytes(
                sh,
                BStr::new(b"OPTARG"),
                Some(BStr::new(argument.as_slice())),
                VariableAttributes::NONE,
            )?;
        } else {
            unset_bytes(sh, BStr::new(b"OPTARG"))?;
        }
        break 'scan;
    }

    if done != 0 {
        unset_bytes(sh, BStr::new(b"OPTARG"))?;
    }

    let index = next as c_int + 1;
    setvarint_bytes(
        sh,
        BStr::new(b"OPTIND"),
        index as i64,
        VariableAttributes::NONE,
        CallbackPolicy::Suppress,
    )?;
    set_bytes(
        sh,
        optvar,
        Some(BStr::new(&[option])),
        VariableAttributes::NONE,
    )?;
    sh.options.shellparam.optoff = cursor.map_or(-1, |(_, at)| at as c_int);
    sh.options.shellparam.optind = index;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::lock;
    use crate::var::lookup_bytes;

    fn value(sh: &mut Shell, name: &str) -> String {
        lookup_bytes(sh, BStr::new(name)).map_or_else(String::new, |value| {
            String::from_utf8_lossy(&value).into_owned()
        })
    }

    // [spec:posix:req:builtin.getopts.optarg/test]
    #[test]
    fn a_scan_runs_across_invocations() {
        let _guard = lock();
        let words = ["getopts", "ab:", "o", "-a", "-bVAL", "rest"];
        let args: Vec<&BStr> = words.iter().map(|word| BStr::new(*word)).collect();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        shell.options.shellparam.optind = 1;
        shell.options.shellparam.optoff = -1;

        assert_eq!(
            getoptscmd(&mut shell, &args).unwrap(),
            Flow::Done((0).into())
        );
        assert_eq!(value(&mut shell, "o"), "a");
        assert_eq!(lookup_bytes(&mut shell, BStr::new(b"OPTARG")), None);
        assert_eq!(
            getoptscmd(&mut shell, &args).unwrap(),
            Flow::Done((0).into())
        );
        assert_eq!(value(&mut shell, "o"), "b");
        assert_eq!(value(&mut shell, "OPTARG"), "VAL");
        assert_ne!(
            getoptscmd(&mut shell, &args).unwrap(),
            Flow::Done((0).into())
        );
        assert_eq!(lookup_bytes(&mut shell, BStr::new(b"OPTARG")), None);
        assert_eq!(value(&mut shell, "OPTIND"), "3");
    }

    // [spec:posix:req:builtin.getopts.stderr-diagnostic/test]
    #[test]
    fn diagnostics_obey_leading_colon() {
        let _guard = lock();
        let words = ["getopts", ":a", "o", "-z"];
        let args: Vec<&BStr> = words.iter().map(|word| BStr::new(*word)).collect();
        let mut shell = Shell::new(crate::streams::Streams::capture().unwrap());
        shell.options.set_arg0(BStr::new(b"my-program"));
        shell.options.shellparam.optind = 1;
        shell.options.shellparam.optoff = -1;

        assert_eq!(
            getoptscmd(&mut shell, &args).unwrap(),
            Flow::Done((0).into())
        );
        assert_eq!(value(&mut shell, "o"), "?");
        assert_eq!(value(&mut shell, "OPTARG"), "z");

        let loud_words = ["getopts", "a", "o", "-z"];
        let loud_args: Vec<&BStr> = loud_words.iter().map(|word| BStr::new(*word)).collect();
        shell.options.shellparam.optind = 1;
        shell.options.shellparam.optoff = -1;
        assert_eq!(
            getoptscmd(&mut shell, &loud_args).unwrap(),
            Flow::Done((0).into())
        );
        assert_eq!(
            shell.take_captured_stderr().unwrap(),
            BString::from("my-program: Illegal option -z\n")
        );
    }
}
