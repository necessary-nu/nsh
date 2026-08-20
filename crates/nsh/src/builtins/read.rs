//! `read`.
//!
//! Port of `readcmd` from `src/miscbltin.c`.
//!
//! Splitting the line it read into fields is `crate::expand`'s
//! `ifsbreakup` -- the same field splitting an unquoted expansion gets,
//! which is why `read` honours `IFS` without knowing what `IFS` is.

// [spec:nsh:req:idiom.operation-modes]
// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};

use crate::descriptors::LogicalDescriptor;
use crate::evaluation::Flow;
use crate::expand::ExpandedFields;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/* glibc <limits.h> */
const MAX_MULTIBYTE_LENGTH: usize = 16;

/// `readcmd`'s `CHECKSTRSPACE((MB_LEN_MAX > 16 ? MB_LEN_MAX : 16) + 4, p)`.
///
/// `getmbc` no longer writes through a cursor this frame makes room for --
/// it has its own scratch and hands back the bytes to append -- so this is
/// not a reservation any more. It survives as the assertion bound on what
/// `getmbc` may return, and the number is still the C's for the reason it
/// always was: with `mode` 0 it puts the character's bytes at `out + 2`
/// and the closing length and marker at `out + 2 + ml` and `out + 3 + ml`,
/// which for `ml == MB_LEN_MAX` is the twentieth byte and not one fewer.
const READ_MULTIBYTE_CAPACITY: usize = (if MAX_MULTIBYTE_LENGTH > 16 {
    MAX_MULTIBYTE_LENGTH
} else {
    16
}) + 4;

fn append_read_byte(line: &mut BString, input: crate::syntax::InputUnit) {
    if input.is(crate::parser::LEGACY_ESCAPE)
        || input.is(crate::parser::LEGACY_MULTIBYTE)
        || input.is(crate::parser::LEGACY_QUOTE)
    {
        line.push(crate::parser::LEGACY_ESCAPE);
    }
    line.push(input.expect_byte());
}

// [spec:nsh:req:idiom.jobs-startup-control-flow]
fn read_input_line(
    shell: &mut Shell,
    delimiter: u8,
    raw: bool,
    prompt_for_continuation: bool,
) -> Result<(BString, ExitStatus), Error> {
    let result = crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::push_standard_input(shell);
        let mut line = BString::default();
        let mut region_start = 0_usize;
        let mut escaped_region_end = None;
        let mut status = ExitStatus::SUCCESS;

        loop {
            let input = if delimiter == b'\0' {
                crate::input::read_input_unit_preserving_nul(shell)?
            } else {
                crate::input::read_input_unit(shell)?
            };
            if input == crate::syntax::InputUnit::EndOfInput {
                status = ExitStatus::FAILURE;
                break;
            }
            if input.is(b'\0') && delimiter != b'\0' {
                continue;
            }

            let mut scratch = [0; crate::parser::MULTIBYTE_OUTPUT_CAPACITY];
            let multibyte_len = crate::parser::read_multibyte_character(
                shell,
                input,
                &mut scratch,
                crate::parser::MultibyteMode::Framed,
            )?;
            if multibyte_len != 0 {
                debug_assert!(multibyte_len <= READ_MULTIBYTE_CAPACITY);
                line.extend_from_slice(&scratch[..multibyte_len]);
            } else if escaped_region_end.is_some() {
                if input.is(b'\n') {
                    if prompt_for_continuation {
                        let ps2 = crate::variables::continuation_prompt_value(shell);
                        shell.write_output(OutputDestination::Stderr, &ps2)?;
                    }
                } else {
                    append_read_byte(&mut line, input);
                }
            } else if !raw && input.is(b'\\') {
                escaped_region_end = Some(line.len());
                continue;
            } else if input.is(delimiter) {
                break;
            } else {
                append_read_byte(&mut line, input);
            }

            if let Some(region_end) = escaped_region_end.take() {
                crate::expand::record_split_region(
                    &mut shell.expand,
                    region_start,
                    region_end,
                    false,
                );
                region_start = line.len();
            }
        }
        Ok::<_, Error>((line, status, region_start))
    });

    let (line, status, region_start) = result?;
    crate::expand::record_split_region(&mut shell.expand, region_start, line.len(), false);
    Ok((line, status))
}

// ---------------------------------------------------------------------

/* Handle one line of the read command.
 *  more fields than variables -> remainder shall be part of last variable.
 *  less fields than variables -> remaining variables unset.
 *
 *  @param line complete line of input
 *  @param ac argument count
 *  @param ap argument (variable) list
 *  @param len length of line including trailing '\0'
 */
// [spec:dash:sem:miscbltin.readcmd-handle-line-fn]
// [spec:posix:req:builtin.read.ifs-empty]
// [spec:posix:req:builtin.read.field-splitting-modified]
// [spec:posix:req:builtin.read.field-splitting-leftover]
// [spec:posix:req:builtin.read.var-assignment-order]
// [spec:posix:req:builtin.read.unprocessed-vars-empty]
// [spec:posix:thm:builtin.read.single-var-unsplit]
// [spec:posix:req:builtin.read.affects-current-environment]
// [spec:posix:req:builtin.read.variable-set-error]
// [spec:posix:def:builtin.read.operand-var]
// [spec:posix:sem:builtin.read.operand-var-locale]
// [spec:posix:req:builtin.read.env]
fn assign_read_fields(shell: &mut Shell, line: &mut BString, names: &[&BStr]) -> Result<(), Error> {
    let mut expanded_fields = ExpandedFields::new();

    /* An owned line already carries its bounds and there is nothing to reserve;
     * the fields `ifsbreakup` builds copy out of it rather than pointing
     * into it, so the line only has to outlive that one call. */
    crate::expand::split_fields(shell, line, names.len(), &mut expanded_fields);
    crate::expand::clear_split_regions(&mut shell.expand);

    /* The C walks the names and the fields with two cursors that advance
     * together, so the field for a name is the field at its index; a name
     * past the last field is the "nullify remaining arguments" case. */
    for (index, name) in names.iter().enumerate() {
        match expanded_fields.fields.get_mut(index) {
            None => {
                crate::variables::set_bytes(
                    shell,
                    name,
                    Some(BStr::new(b"")),
                    crate::variables::VariableAttributes::NONE,
                )?;
            }
            Some(field) => {
                /* set variable to field */
                field.remove_escapes();
                crate::variables::set_bytes(
                    shell,
                    name,
                    Some(field.as_bstr()),
                    crate::variables::VariableAttributes::NONE,
                )?;
            }
        }
    }
    Ok(())
}

/*
 * The read builtin.  The -e option causes backslashes to escape the
 * following character. The -p option followed by an argument prompts
 * with the argument.
 *
 * This uses unbuffered input, which may be avoidable in some cases.
 */

// [spec:dash:sem:miscbltin.readcmd-fn]
// [spec:posix:syn:builtin.read.syn]
// [spec:posix:req:builtin.read.logical-line]
// [spec:posix:req:builtin.read.backslash-escape]
// [spec:posix:req:builtin.read.backslash-line-continuation]
// [spec:posix:req:builtin.read.continuation-prompt]
// [spec:posix:req:builtin.read.env-ps2]
// [spec:posix:req:builtin.read.end-of-file]
// [spec:posix:req:builtin.read.env-nlspath]
// [spec:posix:req:builtin.read.exit-status]
// [spec:posix:req:builtin.read.interfaces]
// [spec:posix:req:builtin.read.option-d]
// [spec:posix:req:builtin.read.option-r]
// [spec:posix:req:builtin.read.stderr]
// [spec:posix:req:builtin.read.stdin]
// [spec:posix:req:builtin.read.terminating-delimiter-removed]
// [spec:posix:req:builtin.read.utility-syntax-guidelines]
// [spec:nsh:req:idiom.lexer-tokens]
// [spec:nsh:def:idiom.logical-descriptors]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut prompt: Option<BString>;
    let mut raw = false;
    let mut delimiter = b'\n';

    prompt = None;
    let mut option_scan = crate::options::Options::new(args);
    while let Some(option) = option_scan.next(&mut shell.diagnostics(), b"d:p:r")? {
        match option {
            b'd' => delimiter = option_scan.arg().first().copied().unwrap_or(b'\0'),
            b'p' => prompt = Some(option_scan.arg().to_owned()),
            _ => raw = true,
        }
    }
    if let Some(prompt) = &prompt {
        if shell
            .descriptors
            .get(LogicalDescriptor::STDIN)
            .as_ref()
            .is_some_and(nsh_platform::is_terminal)
        {
            shell.write_output(OutputDestination::Stderr, prompt)?;
        }
    }
    // [spec:nsh:def:idiom.shell-options]
    let prompt_for_continuation = shell
        .options
        .enabled(crate::options::ShellOption::Interactive)
        && shell
            .descriptors
            .get(LogicalDescriptor::STDIN)
            .as_ref()
            .is_some_and(nsh_platform::is_terminal);
    let names = option_scan.operands();
    if names.is_empty() {
        return Err(shell.diagnostics().shell_error(b"arg count"));
    }

    let (mut line, status) = read_input_line(shell, delimiter, raw, prompt_for_continuation)?;
    assign_read_fields(shell, &mut line, names)?;
    Ok(Flow::Done(status))
}
