//! What `set -x` writes before a command runs.
//!
//! Two writers, because the two halves of a traced command are not the
//! same kind of text. An assignment reaches here spelled the way the
//! script wrote it; an argument reaches here as the result of expansion
//! and, in Bash mode, is quoted so it reads back as itself.

use crate::context::Shell;
use crate::error::Error;
use crate::expand::ExpandedField;
use crate::output::OutputDestination;

/// Trace the assignment words of a command, verbatim.
///
/// [`write_fields`] applies Bash's trace quoting, which is right
/// for an expanded argument and wrong here: an assignment reaches the
/// trace already spelled the way the script wrote it, so quoting it a
/// second time reports `'sp1+=(2)'` for a line that read `sp1+=(2)`.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(super) fn write_assignments(
    shell: &mut Shell,
    dest: OutputDestination,
    list: &[ExpandedField],
    mut already_printed: bool,
) -> Result<bool, Error> {
    for field in list {
        let mut record = Vec::new();
        if already_printed {
            record.push(b' ');
        }
        record.extend_from_slice(field.as_bstr());
        shell.write_output(dest, &record)?;
        already_printed = true;
    }
    Ok(already_printed)
}

// [spec:dash:sem:eval.eprintlist-fn]
pub(super) fn write_fields(
    shell: &mut Shell,
    dest: OutputDestination,
    list: &[ExpandedField],
    mut already_printed: bool,
) -> Result<bool, Error> {
    /* Bash quotes a traced word that would not read back as itself, so
     * `sh -c 'echo 2'` shows one argument rather than two words. dash
     * writes the field as it stands and POSIX mode keeps doing so. */
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let locale = shell.locale.clone();
    for field in list {
        let mut record = Vec::new();
        if already_printed {
            record.push(b' ');
        }
        if bash {
            record.extend_from_slice(&crate::escape::bash::trace_quote(&locale, field.as_bstr()));
        } else {
            record.extend_from_slice(field.as_bstr());
        }
        already_printed = true;
        shell.write_output(dest, &record)?;
    }

    Ok(already_printed)
}
