//! The shell call stack behind `FUNCNAME`, `BASH_SOURCE`, `BASH_LINENO`
//! and the `caller` built-in.
//!
//! Bash's three arrays are one stack read three ways, and they do not
//! line up the way a reader expects: `BASH_SOURCE[i]` is the file the
//! function named by `FUNCNAME[i]` was *defined* in, while
//! `BASH_LINENO[i]` is the line the *call* was written on -- a line in
//! `BASH_SOURCE[i + 1]`, not in `BASH_SOURCE[i]`. Keeping one frame type
//! that carries all three facts is what stops that skew becoming three
//! separate bookkeeping bugs.
//!
//! The arrays are materialised into the variable table on every push and
//! pop rather than computed when read. Reads go through parameter
//! expansion, arithmetic, `${#x}`, `${x[@]}` and `${x:1}`, and a value
//! that is an ordinary indexed array answers all of them without the
//! expansion pipeline knowing this module exists.
//!
//! The other two names Bash publishes about a call, `BASH_ARGC` and
//! `BASH_ARGV`, keep their storage beside the clocks in
//! [`super::special`], because their bottom frame belongs to no call:
//! the reference installs it on the first read that asks, from the
//! shell's own parameters. Everything above that bottom frame is pushed
//! and popped from here, by the same three functions that move the other
//! three arrays -- and *what* a frame contributes is not uniform:
//!
//! * a dot script contributes the word that named it, always;
//! * a function call contributes its own arguments, but only under
//!   `shopt -s extdebug`;
//! * a plain function call contributes nothing at all.

use std::collections::BTreeMap;

use bstr::{BStr, BString};

use super::value::{VariableKind, VariableValue};
use super::{VariableAttributes, VariableState, arrays};
use crate::context::Shell;
use crate::options::Dialect;

/// What Bash calls the frame below every file: the shell itself.
const MAIN: &[u8] = b"main";

/// The three arrays this module owns, in the order they are refreshed.
const ARRAY_NAMES: [&[u8]; 3] = [b"FUNCNAME", b"BASH_SOURCE", b"BASH_LINENO"];

/// The one of the three the reference leaves *declared* rather than
/// assigned when it is empty.
const DECLARED_WHEN_EMPTY: &[u8] = ARRAY_NAMES[0];

/// What `BASH_SOURCE` names for a body the shell read before any file
/// was opened -- from standard input, or from `-c`.
///
/// It is `$0` and not `main`. `main` is `FUNCNAME`'s bottom frame, which
/// is a different thing and is spelled that way there. Measured on the
/// pinned 5.3.15: a function defined on standard input reports
/// `BASH_SOURCE[0]` as `MYNAME` under `exec -a MYNAME bash -c ...` and as
/// `zero0` under `bash -c '...' zero0`, so it follows the name the shell
/// was invoked under rather than the path the binary was found at.
fn invocation_source(shell: &Shell) -> BString {
    shell
        .options
        .argument_zero()
        .map_or_else(|| BString::from(MAIN), BStr::to_owned)
}

/// Whether a frame is a function call or a dot script.
///
/// The distinction is not cosmetic: `FUNCNAME` stays unset while only
/// dot scripts are on the stack, which is the quirk
/// `spec/introspect.test.sh` records for `. file` at the top level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameKind {
    Function,
    Source,
}

/// One entry of the call stack.
#[derive(Clone, Debug)]
pub(crate) struct CallFrame {
    kind: FrameKind,
    /// `FUNCNAME[i]`: the function's name, or `source` for a dot script.
    name: BString,
    /// `BASH_SOURCE[i]`: where this frame's commands were read from.
    source: BString,
    /// `BASH_LINENO[i]`: the line the call was written on.
    line: i32,
    /// Whether this frame put a frame on the `BASH_ARGV` stack, and so
    /// owes it a pop. Not every frame does: see the module header.
    arguments: bool,
}

/// The frames, the script the shell was started with, and where each
/// function was defined.
pub(crate) struct CallStack {
    frames: Vec<CallFrame>,
    /// The command file named on the command line, when there is one.
    ///
    /// Reading from standard input or `-c` leaves this `None`, and then
    /// the arrays have no bottom entry at all -- which is exactly what
    /// Bash reports in that mode.
    base: Option<BString>,
    /// The file each defined function's body was read from.
    definitions: BTreeMap<BString, BString>,
}

impl CallStack {
    pub(crate) const fn new() -> Self {
        Self {
            frames: Vec::new(),
            base: None,
            definitions: BTreeMap::new(),
        }
    }

    /// The file whose commands are executing, as `BASH_SOURCE` spells it.
    fn current_source(&self, fallback: BString) -> BString {
        self.frames
            .last()
            .map(|frame| frame.source.clone())
            .or_else(|| self.base.clone())
            .unwrap_or(fallback)
    }

    /// `FUNCNAME`, `BASH_SOURCE`, `BASH_LINENO`, innermost first.
    ///
    /// The bottom entry belongs to `BASH_SOURCE` and `BASH_LINENO`
    /// whenever the shell has a command file, but reaches `FUNCNAME`
    /// only once a function frame is above it.
    fn arrays(&self) -> [Vec<BString>; 3] {
        let mut names = Vec::new();
        let mut sources = Vec::new();
        let mut lines = Vec::new();
        for frame in self.frames.iter().rev() {
            names.push(frame.name.clone());
            sources.push(frame.source.clone());
            lines.push(BString::from(frame.line.to_string()));
        }
        if let Some(base) = &self.base {
            names.push(BString::from(MAIN));
            sources.push(base.clone());
            lines.push(BString::from("0"));
        }
        if !self.frames.iter().any(|f| f.kind == FrameKind::Function) {
            names.clear();
        }
        [names, sources, lines]
    }

    /// `${BASH_LINENO[index]}`, for the `caller` built-in.
    pub(crate) fn call_line(&self, index: usize) -> Option<BString> {
        let [_, _, lines] = self.arrays();
        lines.get(index).cloned()
    }

    /// `${FUNCNAME[index]}` as `caller` reads it, which is before the
    /// `FUNCNAME` visibility quirk applies.
    pub(crate) fn frame_name(&self, index: usize) -> Option<BString> {
        let mut names: Vec<BString> = self
            .frames
            .iter()
            .rev()
            .map(|frame| frame.name.clone())
            .collect();
        if self.base.is_some() {
            names.push(BString::from(MAIN));
        }
        names.get(index).cloned()
    }

    /// `${BASH_SOURCE[index]}`.
    pub(crate) fn frame_source(&self, index: usize) -> Option<BString> {
        let [_, sources, _] = self.arrays();
        sources.get(index).cloned()
    }

    /// How many function and dot-script frames are active.
    pub(crate) fn depth(&self) -> usize {
        self.frames.len()
    }

    /// How many *function* frames are active, which is the narrower
    /// question `BASH_ARGC`'s install asks: the reference installs from
    /// inside a dot script and refuses from inside a function.
    pub(crate) fn function_depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.kind == FrameKind::Function)
            .count()
    }
}

/// How deeply the evaluator is nested, counting every re-entry that
/// spends a stack frame on what it is about to run.
///
/// A call, a dot script and an `eval` cost the same kind of frame and
/// share one ceiling, because they compose: `f() { eval f; }` spends one
/// of each per turn, and two separate ceilings would let it reach a depth
/// neither of them names. The call stack already counts the first two, so
/// only the string re-entries need a counter of their own.
// [spec:nsh:req:idiom.bounded-recursion]
pub(crate) fn evaluation_depth(shell: &Shell) -> usize {
    shell.variables.call_stack.depth() + shell.evaluation.nested_evaluations
}

/// Record the command file the shell was started with.
///
/// Only a named file produces a bottom frame; `sh -c` and a shell reading
/// standard input have none, and Bash reports empty arrays for both.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn set_script_file(shell: &mut Shell, path: &BStr) {
    shell.variables.call_stack.base = Some(path.to_owned());
    refresh(shell);
}

/// Remember where a function's body was read from, for `BASH_SOURCE`.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn record_definition(shell: &mut Shell, name: &BStr) {
    if shell.options.dialect() != Dialect::Bash {
        return;
    }
    let source = shell
        .variables
        .call_stack
        .current_source(invocation_source(shell));
    shell
        .variables
        .call_stack
        .definitions
        .insert(name.to_owned(), source);
}

/// The file a function's body was read from, as `declare -F` reports
/// it under `shopt -s extdebug`.
///
/// A function the shell read before any file was named -- from `-c`, or
/// from standard input -- belongs to the frame Bash calls `main`, which
/// is the same fallback a call to it pushes.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn definition_source(shell: &Shell, name: &BStr) -> BString {
    shell
        .variables
        .call_stack
        .definitions
        .get(name)
        .cloned()
        .unwrap_or_else(|| invocation_source(shell))
}

/// Enter a function call, at `line` of the caller's file.
///
/// The call's own arguments reach `BASH_ARGV` only under
/// `shopt -s extdebug`; without it the reference answers `()` inside a
/// function it has been given arguments for, which is what makes the two
/// halves of this pair separate mechanisms rather than one. They are
/// read from the positional parameters rather than passed in because the
/// caller has already made them the parameters by the time it gets here:
/// that is what `$1` inside the body means.
// [spec:nsh:req:compat.bash.names.call-stack]
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn push_function(shell: &mut Shell, name: &BStr, line: i32) {
    let source = definition_source(shell, name);
    let mut arguments = false;
    if shell.options.shopt(crate::options::BashShopt::ExtDebug) {
        let words = shell.options.positional_parameters.words();
        arguments = super::special::push_call_arguments(shell, words);
    }
    shell.variables.call_stack.frames.push(CallFrame {
        kind: FrameKind::Function,
        name: name.to_owned(),
        source,
        line,
        arguments,
    });
    refresh(shell);
}

/// Enter a dot script, at `line` of the file that named it.
///
/// A dot script's `BASH_ARGV` frame is the word that named it, and it is
/// pushed whether or not `extdebug` is on -- measured on the pinned
/// 5.3.15, where `. lib.sh` from a shell started `-s a b c` reports
/// `BASH_ARGC=([0]="1" [1]="3")` with `BASH_ARGV[0]` the path. It is the
/// *word*, resolved the way `BASH_SOURCE` resolves it: a `PATH`-found
/// name reports the file it was found at in both.
///
/// The reference pushes the operands instead when the dot script is
/// given any, and only under `extdebug`. This shell has no dot-script
/// operands to push -- see
/// `bash.divergences.dot-script-operands-are-positional-parameters` --
/// so there is one shape here rather than two.
// [spec:nsh:req:compat.bash.names.call-stack]
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn push_source(shell: &mut Shell, path: &BStr, line: i32) {
    let arguments = super::special::push_call_arguments(shell, vec![path.to_owned()]);
    shell.variables.call_stack.frames.push(CallFrame {
        kind: FrameKind::Source,
        name: BString::from("source"),
        source: path.to_owned(),
        line,
        arguments,
    });
    refresh(shell);
}

/// Leave the innermost frame.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn pop(shell: &mut Shell) {
    if let Some(frame) = shell.variables.call_stack.frames.pop()
        && frame.arguments
    {
        super::special::pop_call_arguments(shell);
    }
    refresh(shell);
}

/// Write the three arrays back into the variable table.
///
/// An empty stack is published empty rather than unset, because the
/// reference has all three names from the moment it starts: at rest on
/// standard input it prints `declare -a BASH_SOURCE=()` and
/// `declare -a BASH_LINENO=()`, and a script walking `declare -p` sees
/// them. `FUNCNAME` is the one it leaves *declared* with no value at all,
/// which is `VariableState::Declared` here.
///
/// Neither spelling gives `$FUNCNAME` a value: an empty indexed array has
/// no element zero either, so `set -u` still diagnoses all three outside
/// a function, and `${FUNCNAME[@]}` is still silent under it. The
/// difference the two spellings do make is the one `declare -p` prints,
/// which is the whole reason to keep them apart.
///
/// A refusal is dropped rather than raised. The only way to earn one is
/// to make one of the three names read-only, and a call must not fail
/// because the shell could not publish its own introspection.
// [spec:nsh:req:compat.bash.names.call-stack]
pub(crate) fn refresh(shell: &mut Shell) {
    if shell.options.dialect() != Dialect::Bash {
        return;
    }
    let values = shell.variables.call_stack.arrays();
    for (name, elements) in ARRAY_NAMES.into_iter().zip(values) {
        let declared = elements.is_empty() && name == DECLARED_WHEN_EMPTY;
        let name = BStr::new(name);
        let mut value = VariableValue::empty(VariableKind::Indexed);
        for (index, element) in elements.iter().enumerate() {
            value.set_indexed(index as u64, BStr::new(element.as_slice()));
        }
        // The shell is landing its own bookkeeping, not performing a user
        // assignment, so a read-only mark on one of these names must not
        // leave the call stack stale.
        drop(arrays::store(
            shell,
            name,
            value,
            VariableAttributes::NONE,
            arrays::ReadOnlyGuard::Declaration,
        ));
        if declared && let Some(entry) = shell.variables.entries.get_mut(name) {
            entry.state = VariableState::Declared(VariableKind::Indexed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ShellOption;
    use crate::test_support::lock;
    use crate::variables::lookup_bytes;

    fn bash_shell() -> Shell {
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        shell.options.set(ShellOption::Bash, true);
        shell
    }

    /// `BASH_ARGC` as the stack it is, outermost frame last.
    fn argument_counts(shell: &mut Shell) -> Vec<BString> {
        let name = BStr::new("BASH_ARGC");
        super::super::value::variable_value_owned(shell, name)
            .as_ref()
            .map(arrays::elements)
            .unwrap_or_default()
    }

    /// A shell that has been through the dialect's publish, which is
    /// what puts `BASH_ARGC` and `BASH_ARGV` on the table at all: the
    /// pushes below write into those entries and decline to resurrect a
    /// name that is not there.
    fn published_bash_shell() -> Shell {
        let mut shell = bash_shell();
        super::super::special::dialect_changed(&mut shell);
        shell
    }

    /// A function frame reports the file its body was read from, and the
    /// line its call was written on.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn a_function_frame_reports_definition_and_call() {
        let _g = lock();
        let shell = &mut bash_shell();
        record_definition(shell, BStr::new("f"));
        push_function(shell, BStr::new("f"), 12);

        assert_eq!(
            lookup_bytes(shell, BStr::new("FUNCNAME")),
            Some(BString::from("f"))
        );
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_SOURCE")),
            Some(BString::from("main"))
        );
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_LINENO")),
            Some(BString::from("12"))
        );

        pop(shell);
        assert_eq!(lookup_bytes(shell, BStr::new("FUNCNAME")), None);
        assert_eq!(lookup_bytes(shell, BStr::new("BASH_SOURCE")), None);
    }

    /// A dot script contributes to `BASH_SOURCE` while leaving
    /// `FUNCNAME` unset, and a function called from it is reported as
    /// defined in that file.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn a_dot_script_leaves_funcname_unset() {
        let _g = lock();
        let shell = &mut bash_shell();
        push_source(shell, BStr::new("lib.sh"), 4);
        assert_eq!(lookup_bytes(shell, BStr::new("FUNCNAME")), None);
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_SOURCE")),
            Some(BString::from("lib.sh"))
        );

        record_definition(shell, BStr::new("g"));
        pop(shell);
        push_function(shell, BStr::new("g"), 9);
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_SOURCE")),
            Some(BString::from("lib.sh"))
        );
        assert_eq!(shell.variables.call_stack.depth(), 1);
    }

    /// A command file adds the bottom `main` entry, which reaches
    /// `FUNCNAME` only once a function frame sits above it.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn a_command_file_adds_the_main_frame() {
        let _g = lock();
        let shell = &mut bash_shell();
        set_script_file(shell, BStr::new("script.sh"));
        assert_eq!(lookup_bytes(shell, BStr::new("FUNCNAME")), None);
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_SOURCE")),
            Some(BString::from("script.sh"))
        );

        record_definition(shell, BStr::new("f"));
        push_function(shell, BStr::new("f"), 21);
        assert_eq!(
            shell.variables.call_stack.frame_name(1),
            Some(BString::from("main"))
        );
        assert_eq!(
            shell.variables.call_stack.call_line(0),
            Some(BString::from("21"))
        );
        assert_eq!(
            shell.variables.call_stack.frame_source(1),
            Some(BString::from("script.sh"))
        );
    }

    /// A dot script's frame is the word that named it, and it is gone
    /// again once the frame is popped -- which is what leaves the
    /// install's own frame standing underneath.
    // [spec:nsh:req:compat.bash.names.call-stack/test]
    #[test]
    fn a_dot_script_pushes_the_naming_word() {
        let _g = lock();
        let shell = &mut published_bash_shell();
        push_source(shell, BStr::new("lib.sh"), 4);
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_ARGV")),
            Some(BString::from("lib.sh"))
        );
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_ARGC")),
            Some(BString::from("1"))
        );

        pop(shell);
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_ARGC")),
            Some(BString::from("0"))
        );
    }

    /// A function call pushes its arguments only under `extdebug`, and
    /// what it pushes is the positional parameters the caller has
    /// already installed for the body.
    // [spec:nsh:req:compat.bash.names.call-stack/test]
    #[test]
    fn a_traced_function_frame_pushes_its_arguments() {
        let _g = lock();
        let shell = &mut published_bash_shell();
        crate::options::set_positional_parameters(shell, &[BStr::new("a"), BStr::new("b")]);

        // An untraced call pushes nothing, and a read taken inside one
        // installs nothing either, so both names stay the empty arrays
        // the dialect published -- an empty array has no element zero.
        push_function(shell, BStr::new("f"), 1);
        assert_eq!(argument_counts(shell), Vec::<BString>::new());

        // The read taken once the frame is gone is the install, and it
        // is the only frame there is: one count, for the shell's own two
        // parameters.
        pop(shell);
        assert_eq!(argument_counts(shell), vec![BString::from("2")]);

        shell.options.set_bash_option(BStr::new("extdebug"), true);
        push_function(shell, BStr::new("f"), 1);
        assert_eq!(
            argument_counts(shell),
            vec![BString::from("2"), BString::from("2")]
        );
        // Innermost argument first, which is the order the whole stack
        // runs in.
        assert_eq!(
            lookup_bytes(shell, BStr::new("BASH_ARGV")),
            Some(BString::from("b"))
        );

        // The pop takes the call's frame and leaves the install's.
        pop(shell);
        assert_eq!(argument_counts(shell), vec![BString::from("2")]);
    }

    /// The POSIX dialect never publishes the arrays.
    // [spec:nsh:req:compat.bash.traps-introspection/test]
    #[test]
    fn the_posix_dialect_publishes_nothing() {
        let _g = lock();
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        push_function(shell, BStr::new("f"), 3);
        assert_eq!(lookup_bytes(shell, BStr::new("FUNCNAME")), None);
        assert_eq!(lookup_bytes(shell, BStr::new("BASH_SOURCE")), None);
    }
}
