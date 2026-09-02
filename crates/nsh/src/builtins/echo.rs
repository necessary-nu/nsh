//! The `echo` builtin.
//!
//! Port of the `echo` half of `src/bltin/printf.c`.
//! Rules: `docs/spec/port/src/bltin/printf.md`.
//!
//! The other half of that C file is the `printf` utility, which lives in
//! `builtins/printf.rs`. Reading a pattern at runtime is what POSIX
//! defines that utility to do, and
//! `[dec:nsh:printf-is-parsed-not-interpreted]` sanctions it there and
//! nowhere else: nothing outside `builtins::printf` formats a value by a
//! pattern chosen at runtime. `echo` is on the far side of that line.
//! Output is an `io::Write` and what this file writes goes through
//! `write!` at call sites where the arguments already have types.
//!
//! What the split removed is the C's machinery rather than the utility:
//! `mklong`, the `PF`/`ASPF` arity switch and the `snprintf` bridge
//! behind them are gone, and the specification is parsed into typed
//! fields instead. `print_escape_str` kept only what `echo` needs -- the
//! C passed it a format string of which two bytes ever mattered, and it
//! takes those directly now.
//!
//! The escape decoding itself is [`crate::escape`], because the parser
//! shares it: `$\'...\'` is the same decoder with `mbchar` set.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};

use crate::escape::append_escape;
use crate::evaluation::Flow;
use crate::output::OutputDestination;

/// Expand `echo`'s escapes and write the result, followed by `separator`
/// unless a `\c` stopped the conversion.
///
/// The C took a format string and three of its bytes meant something:
/// `f[1]` said whether the conversion character sat right after the `%`,
/// and `f[2]` was the byte to append — `echo`'s space or its closing
/// newline. `echo` is the only caller left and it only ever passed `%s`,
/// `%s ` or `%s\n`, so it passes the byte itself.
// [spec:dash:sem:printf.print-escape-str-fn]
fn write_escaped_text(
    shell: &mut Shell,
    separator: u8,
    escapes: bool,
    text: &BStr,
) -> Result<bool, Error> {
    /* The C's `q` is a cursor into the stack block and `stackblock()` its
     * base.  Both are this buffer: `len` is its length and `q[-1]` its
     * last byte. */
    let mut buffer = BString::default();

    /* dash's `echo` always decodes; Bash's decodes only for `-e`, and a
     * script that prints a backslash relies on which one it is talking
     * to. The dialect answers that, so the decision arrives here as a
     * flag rather than being re-derived per word. */
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    let stopped = if escapes {
        append_escape(&shell.locale, text, &mut buffer)
    } else {
        buffer.extend_from_slice(text);
        false
    };
    shell.write_output(OutputDestination::Stdout, &buffer)?;
    if !stopped && separator != 0 {
        shell.write_output(OutputDestination::Stdout, &[separator])?;
    }

    Ok(stopped)
}

// [spec:dash:sem:printf.echocmd-fn]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* The C picked between the formats `"%s\n"`, `"%s"` and `"%s "`; all
     * that ever differed was the byte after the conversion, so what is
     * chosen here is that byte. `-n` closes with nothing. */
    let mut last: u8 = b'\n';

    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let mut escapes = !bash;
    let mut words = &args[1..];
    if bash {
        /* Bash reads a run of words made only of `n`, `e` and `E` as
         * options; dash reads exactly one `-n` and nothing else. */
        while let Some(word) = words.first() {
            let Some(letters) = word.strip_prefix(b"-").filter(|rest| !rest.is_empty()) else {
                break;
            };
            if !letters
                .iter()
                .all(|letter| matches!(letter, b'n' | b'e' | b'E'))
            {
                break;
            }
            for letter in letters {
                match letter {
                    b'n' => last = 0,
                    b'e' => escapes = true,
                    _ => escapes = false,
                }
            }
            words = &words[1..];
        }
    } else if words.first().is_some_and(|w| &w[..] == b"-n") {
        words = &words[1..];
        last = 0;
    }

    let mut index = 0usize;
    loop {
        let mut separator: u8 = b' ';
        let selected_word = words.get(index);

        // if (!s || !*++argv) — `++argv` is not evaluated when s is NULL.
        if selected_word.is_none() || {
            index += 1;
            words.get(index).is_none()
        } {
            separator = last;
        }

        let stopped = write_escaped_text(
            shell,
            separator,
            escapes,
            selected_word.copied().unwrap_or(BStr::new(b"")),
        )?;

        if stopped || words.get(index).is_none() {
            break;
        }
    }
    Ok(Flow::Done((0).into()))
}
