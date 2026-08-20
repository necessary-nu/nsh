//! The script-visible `getopts` scanner.
//!
//! Its persistent cursor remains in `shellparam`; each invocation scans an
//! owned snapshot of either the positional parameters or explicit operands.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::options::Options;
use crate::output::OutputDestination;
use crate::variables::{
    CallbackPolicy, VariableAttributes, set_bytes, set_integer_bytes, unset_bytes,
};

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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut option_scan = Options::new(args);
    option_scan.next(&mut shell.diagnostics(), b"")?;
    let operands = option_scan.operands();
    if operands.len() < 2 {
        return Err(shell
            .diagnostics()
            .shell_error(b"Usage: getopts optstring var [arg...]"));
    }

    let words: Vec<BString> = if operands.len() == 2 {
        let words = shell.options.positional_parameters.words();
        if shell.options.positional_parameters.option_index
            > shell.options.positional_parameters.parameter_count + 1
        {
            shell.options.positional_parameters.option_index = 1;
            shell.options.positional_parameters.option_offset = None;
        }
        words
    } else {
        if shell.options.positional_parameters.option_index > operands.len() - 1 {
            shell.options.positional_parameters.option_index = 1;
            shell.options.positional_parameters.option_offset = None;
        }
        operands[2..]
            .iter()
            .map(|word| (*word).to_owned())
            .collect()
    };

    Ok(Flow::Done(
        i32::from(getopts(shell, operands[0], operands[1], &words)?).into(),
    ))
}

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
    shell: &mut Shell,
    optstr: &BStr,
    optvar: &BStr,
    words: &[BString],
) -> Result<bool, Error> {
    let mut option = b'?';
    let mut done = false;
    let mut next = shell
        .options
        .positional_parameters
        .option_index
        .saturating_sub(1);
    let offset = shell.options.positional_parameters.option_offset;

    let mut cursor = if next > 0
        && offset.is_some_and(|offset| words.get(next - 1).is_some_and(|word| word.len() >= offset))
    {
        Some((next - 1, offset.expect("validated option offset")))
    } else {
        None
    };

    'scan: {
        if cursor.is_none() || cursor.is_some_and(|(word, at)| at >= words[word].len()) {
            let Some(word) = words.get(next) else {
                done = true;
                cursor = None;
                break 'scan;
            };
            if word.first() != Some(&b'-') || word.len() == 1 {
                done = true;
                cursor = None;
                break 'scan;
            }
            next += 1;
            if word.as_slice() == b"--" {
                done = true;
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
                    shell,
                    BStr::new(b"OPTARG"),
                    Some(BStr::new(&[option])),
                    VariableAttributes::NONE,
                )?;
            } else {
                let mut message = shell
                    .options
                    .argument_zero()
                    .unwrap_or_else(|| BStr::new(b"sh"))
                    .to_vec();
                message.extend_from_slice(b": Illegal option -");
                message.push(option);
                message.push(b'\n');
                shell.write_output(OutputDestination::Stderr, &message)?;
                unset_bytes(shell, BStr::new(b"OPTARG"))?;
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
                        shell,
                        BStr::new(b"OPTARG"),
                        Some(BStr::new(&[option])),
                        VariableAttributes::NONE,
                    )?;
                    option = b':';
                } else {
                    let mut message = shell
                        .options
                        .argument_zero()
                        .unwrap_or_else(|| BStr::new(b"sh"))
                        .to_vec();
                    message.extend_from_slice(b": No arg for -");
                    message.push(option);
                    message.extend_from_slice(b" option\n");
                    shell.write_output(OutputDestination::Stderr, &message)?;
                    unset_bytes(shell, BStr::new(b"OPTARG"))?;
                    option = b'?';
                }
                break 'scan;
            };
            set_bytes(
                shell,
                BStr::new(b"OPTARG"),
                Some(BStr::new(argument.as_slice())),
                VariableAttributes::NONE,
            )?;
        } else {
            unset_bytes(shell, BStr::new(b"OPTARG"))?;
        }
        break 'scan;
    }

    if done {
        unset_bytes(shell, BStr::new(b"OPTARG"))?;
    }

    let index = next + 1;
    set_integer_bytes(
        shell,
        BStr::new(b"OPTIND"),
        i64::try_from(index).unwrap_or(i64::MAX),
        VariableAttributes::NONE,
        CallbackPolicy::Suppress,
    )?;
    set_bytes(
        shell,
        optvar,
        Some(BStr::new(&[option])),
        VariableAttributes::NONE,
    )?;
    shell.options.positional_parameters.option_offset = cursor.map(|(_, at)| at);
    shell.options.positional_parameters.option_index = index;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock;
    use crate::variables::lookup_bytes;

    fn value(shell: &mut Shell, name: &str) -> String {
        lookup_bytes(shell, BStr::new(name)).map_or_else(String::new, |value| {
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
        shell.options.positional_parameters.option_index = 1;
        shell.options.positional_parameters.option_offset = None;

        assert_eq!(run(&mut shell, &args).unwrap(), Flow::Done((0).into()));
        assert_eq!(value(&mut shell, "o"), "a");
        assert_eq!(lookup_bytes(&mut shell, BStr::new(b"OPTARG")), None);
        assert_eq!(run(&mut shell, &args).unwrap(), Flow::Done((0).into()));
        assert_eq!(value(&mut shell, "o"), "b");
        assert_eq!(value(&mut shell, "OPTARG"), "VAL");
        assert_ne!(run(&mut shell, &args).unwrap(), Flow::Done((0).into()));
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
        shell.options.positional_parameters.option_index = 1;
        shell.options.positional_parameters.option_offset = None;

        assert_eq!(run(&mut shell, &args).unwrap(), Flow::Done((0).into()));
        assert_eq!(value(&mut shell, "o"), "?");
        assert_eq!(value(&mut shell, "OPTARG"), "z");

        let loud_words = ["getopts", "a", "o", "-z"];
        let loud_args: Vec<&BStr> = loud_words.iter().map(|word| BStr::new(*word)).collect();
        shell.options.positional_parameters.option_index = 1;
        shell.options.positional_parameters.option_offset = None;
        assert_eq!(run(&mut shell, &loud_args).unwrap(), Flow::Done((0).into()));
        assert_eq!(
            shell.take_captured_stderr().unwrap(),
            BString::from("my-program: Illegal option -z\n")
        );
    }
}
