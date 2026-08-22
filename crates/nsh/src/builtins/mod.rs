//! The builtins: one module per builtin, and the table that names them.
//!
//! `builtins::<name>` is the whole organising idea -- a builtin's entry
//! point and the helpers only it uses live in the module named after it,
//! while the machinery it drives (the variable table, job control, the
//! alias table, the parser) stays in the module that owns that machinery
//! and is called from here. Where a builtin's name is a Rust keyword the
//! module is a raw identifier, so `type` is `builtins::r#type`: the module
//! is named after the builtin even when the language would rather it were
//! not.
//!
use bstr::BStr;

/// A builtin's entry point.
///
/// The words the shell expanded, `argv[0]` first, borrowed from the
/// caller's storage rather than from any shell state -- which is the
/// constraint `[dec:nsh:public-surface]` records, because a builtin that
/// re-enters evaluation (`.`, `eval`, `fc`) has to be able to hand the
/// shell straight back.
///
/// The C's `int (*)(int, char **)` is gone, and with it the count: a
/// slice carries its own length, and no builtin has to be told twice.
///
/// The status is a `Result` because a builtin that fails hands its
/// diagnostic back rather than jumping out with it
/// ([dec:nsh:errors-are-values]). The `Err` has already been reported: the
/// bytes went to stderr where dash writes them, and the value is what the
/// caller -- and eventually an embedder -- gets to inspect.
///
/// `[dec:nsh:public-surface]` records the destination as
/// `fn(&mut Shell, &[&BStr]) -> Result<ExitStatus, Error>`. This is that
/// signature's receiver and `Result`; the status type belongs to
/// `public-api`.
///
/// The receiver owns all mutable shell state
/// ([dec:nsh:no-ambient-state]), so builtin entry points are ordinary safe
/// functions rather than callbacks into ambient globals.
///
/// The `Ok` side is a [`Flow`] rather than a status because `exit` is a
/// built-in. `exitcmd` used to leave by `exraise(EXEXIT)`, and a table of
/// one function-pointer type is what makes that everybody's business:
/// either every entry can say "the shell is exiting" or `exit` has to
/// keep jumping. Three others need it too, and they need it for the same
/// reason -- `.`, `fc` and `eval` re-enter evaluation, so an `exit` or a
/// `set -e` abort inside them has to travel back out through them. The
/// remaining thirty produce `Flow::Done` and nothing else, which is what
/// the C's `int` said.
pub(crate) type Builtin = fn(
    &mut crate::context::Shell,
    &[&BStr],
) -> Result<crate::evaluation::Flow, crate::error::Error>;

/// Stable identity for the handful of built-ins whose evaluator semantics
/// differ from an ordinary registry dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuiltinId {
    Empty,
    Dot,
    Colon,
    Bracket,
    Alias,
    Bg,
    Break,
    Caller,
    Cd,
    Chdir,
    Command,
    Continue,
    Echo,
    Eval,
    Exec,
    Exit,
    Export,
    False,
    Fc,
    Fg,
    Getopts,
    Hash,
    History,
    Jobs,
    Kill,
    Local,
    Printf,
    DirectoryUpdate,
    Read,
    Readonly,
    Return,
    Set,
    Shift,
    Source,
    Test,
    Times,
    Trap,
    True,
    Type,
    Ulimit,
    Umask,
    Unalias,
    Unset,
    Wait,
    Shopt,
    Declare,
    Builtin,
    Enable,
    Help,
    Let,
    Mapfile,
    Dirs,
    Pushd,
    Popd,
}

/// Typed, independent properties of a built-in command.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BuiltinAttributes {
    special: bool,
    regular: bool,
    assignment: bool,
}

impl BuiltinAttributes {
    const NONE: Self = Self {
        special: false,
        regular: false,
        assignment: false,
    };
    const REGULAR: Self = Self {
        special: false,
        regular: true,
        assignment: false,
    };
    const REGULAR_ASSIGNMENT: Self = Self {
        special: false,
        regular: true,
        assignment: true,
    };
    const SPECIAL: Self = Self {
        special: true,
        regular: true,
        assignment: false,
    };
    const SPECIAL_ASSIGNMENT: Self = Self {
        special: true,
        regular: true,
        assignment: true,
    };

    pub(crate) const fn is_special(self) -> bool {
        self.special
    }

    pub(crate) const fn is_regular(self) -> bool {
        self.regular
    }

    pub(crate) const fn takes_assignments(self) -> bool {
        self.assignment
    }
}

/// Every registry row has a handler; exceptional call signatures are enum
/// variants rather than nullable function pointers.
#[derive(Clone, Copy)]
pub(crate) enum BuiltinHandler {
    Standard(Builtin),
    Eval,
    History,
}

/// One byte-preserving, fully typed built-in registry entry.
// [spec:nsh:req:idiom.builtin-registry]
pub(crate) struct BuiltinSpec {
    id: BuiltinId,
    name: &'static [u8],
    handler: BuiltinHandler,
    attributes: BuiltinAttributes,
}

impl BuiltinSpec {
    pub(crate) const fn id(&self) -> BuiltinId {
        self.id
    }

    pub(crate) fn name(&self) -> &'static BStr {
        BStr::new(self.name)
    }

    pub(crate) const fn handler(&self) -> BuiltinHandler {
        self.handler
    }

    pub(crate) const fn attributes(&self) -> BuiltinAttributes {
        self.attributes
    }
}

/// The words a builtin is handed, out of the fields `evalcommand`
/// expanded.
///
pub fn args(fields: &[crate::expand::ExpandedField]) -> Vec<&BStr> {
    fields
        .iter()
        .map(crate::expand::ExpandedField::as_bstr)
        .collect()
}

pub mod alias;
pub mod r#break;
pub mod builtin;
pub mod caller;
pub mod cd;
pub mod command;
pub mod declare;
pub mod dirs;
pub mod dot;
pub mod echo;
pub mod enable;
pub mod eval;
pub mod exec;
pub mod exit;
pub mod export;
pub mod r#false;
pub mod fc;
pub mod fg;
pub mod getopts;
pub mod hash;
pub mod help;
pub mod history;
pub mod jobs;
pub mod kill;
pub mod r#let;
pub mod local;
pub mod mapfile;
pub mod printf;
pub mod pwd;
pub mod read;
pub mod r#return;
pub mod set;
pub mod shift;
pub mod shopt;
pub mod test;
pub mod times;
pub mod trap;
pub mod r#true;
pub mod r#type;
pub mod ulimit;
pub mod umask;
pub mod unalias;
pub mod unset;
pub mod wait;

/// The nameless row: a command that is only assignments and
/// redirections still runs a builtin, and this is it.
///
/// The C keeps it in `eval.c` beside `evalcommand`, which is the only
/// thing that reaches for it. It is a table row, so it lives with the
/// table.
pub(crate) static EMPTY_BUILTIN: BuiltinSpec = BuiltinSpec {
    id: BuiltinId::Empty,
    name: b"",
    handler: BuiltinHandler::Standard(execute_builtin),
    attributes: BuiltinAttributes::REGULAR,
};

// [spec:dash:sem:eval.bltincmd-fn]
fn execute_builtin(
    shell: &mut crate::context::Shell,
    _args: &[&BStr],
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    /*
     * Preserve exitstatus of a previous possible redirection
     * as POSIX mandates
     */
    Ok(crate::evaluation::Flow::Done(
        shell.evaluation.command_substitution_status,
    ))
}

// [spec:posix:req:builtin.special.supported-and-output]
// [spec:posix:def:builtin.special.term-built-in]
// [spec:posix:req:builtin.special.not-exec-accessible]
// [spec:posix:req:xcu.builtin.regular-permitted]
// [spec:posix:req:xcu.builtin.exec-accessible]
// [spec:posix:req:xcu.intrinsic-utilities]
// [spec:posix:req:xcu.intrinsic.additional-implementation-defined]
pub(crate) static BUILTINS: &[BuiltinSpec] = &[
    BuiltinSpec {
        id: BuiltinId::Dot,
        name: b".",
        handler: BuiltinHandler::Standard(dot::run_dot),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 0
    BuiltinSpec {
        id: BuiltinId::Colon,
        name: b":",
        handler: BuiltinHandler::Standard(r#true::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 1
    BuiltinSpec {
        id: BuiltinId::Bracket,
        name: b"[",
        handler: BuiltinHandler::Standard(test::run),
        attributes: BuiltinAttributes::NONE,
    }, // 2
    BuiltinSpec {
        id: BuiltinId::Alias,
        name: b"alias",
        handler: BuiltinHandler::Standard(alias::run),
        attributes: BuiltinAttributes::REGULAR_ASSIGNMENT,
    }, // 3
    BuiltinSpec {
        id: BuiltinId::Bg,
        name: b"bg",
        handler: BuiltinHandler::Standard(fg::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 4
    BuiltinSpec {
        id: BuiltinId::Break,
        name: b"break",
        handler: BuiltinHandler::Standard(r#break::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 5
    BuiltinSpec {
        id: BuiltinId::Cd,
        name: b"cd",
        handler: BuiltinHandler::Standard(cd::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 6
    BuiltinSpec {
        id: BuiltinId::Chdir,
        name: b"chdir",
        handler: BuiltinHandler::Standard(cd::run),
        attributes: BuiltinAttributes::NONE,
    }, // 7
    BuiltinSpec {
        id: BuiltinId::Command,
        name: b"command",
        handler: BuiltinHandler::Standard(command::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 8
    BuiltinSpec {
        id: BuiltinId::Continue,
        name: b"continue",
        handler: BuiltinHandler::Standard(r#break::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 9
    BuiltinSpec {
        id: BuiltinId::Echo,
        name: b"echo",
        handler: BuiltinHandler::Standard(echo::run),
        attributes: BuiltinAttributes::NONE,
    }, // 10
    BuiltinSpec {
        id: BuiltinId::Eval,
        name: b"eval",
        handler: BuiltinHandler::Eval,
        attributes: BuiltinAttributes::SPECIAL,
    }, // 11
    BuiltinSpec {
        id: BuiltinId::Exec,
        name: b"exec",
        handler: BuiltinHandler::Standard(exec::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 12
    BuiltinSpec {
        id: BuiltinId::Exit,
        name: b"exit",
        handler: BuiltinHandler::Standard(exit::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 13
    BuiltinSpec {
        id: BuiltinId::Export,
        name: b"export",
        handler: BuiltinHandler::Standard(export::run),
        attributes: BuiltinAttributes::SPECIAL_ASSIGNMENT,
    }, // 14
    BuiltinSpec {
        id: BuiltinId::False,
        name: b"false",
        handler: BuiltinHandler::Standard(r#false::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 15
    BuiltinSpec {
        id: BuiltinId::Fc,
        name: b"fc",
        handler: BuiltinHandler::History,
        attributes: BuiltinAttributes::REGULAR,
    }, // 16
    BuiltinSpec {
        id: BuiltinId::Fg,
        name: b"fg",
        handler: BuiltinHandler::Standard(fg::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 17
    BuiltinSpec {
        id: BuiltinId::Getopts,
        name: b"getopts",
        handler: BuiltinHandler::Standard(getopts::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 18
    BuiltinSpec {
        id: BuiltinId::Hash,
        name: b"hash",
        handler: BuiltinHandler::Standard(hash::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 19
    BuiltinSpec {
        id: BuiltinId::History,
        name: b"history",
        handler: BuiltinHandler::Standard(history::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 20
    BuiltinSpec {
        id: BuiltinId::Jobs,
        name: b"jobs",
        handler: BuiltinHandler::Standard(jobs::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 21
    BuiltinSpec {
        id: BuiltinId::Kill,
        name: b"kill",
        handler: BuiltinHandler::Standard(kill::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 22
    BuiltinSpec {
        id: BuiltinId::Local,
        name: b"local",
        handler: BuiltinHandler::Standard(local::run),
        attributes: BuiltinAttributes::SPECIAL_ASSIGNMENT,
    }, // 23
    BuiltinSpec {
        id: BuiltinId::Printf,
        name: b"printf",
        handler: BuiltinHandler::Standard(printf::run),
        attributes: BuiltinAttributes::NONE,
    }, // 24
    BuiltinSpec {
        id: BuiltinId::DirectoryUpdate,
        name: b"pwd",
        handler: BuiltinHandler::Standard(pwd::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 25
    BuiltinSpec {
        id: BuiltinId::Read,
        name: b"read",
        handler: BuiltinHandler::Standard(read::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 26
    BuiltinSpec {
        id: BuiltinId::Readonly,
        name: b"readonly",
        handler: BuiltinHandler::Standard(export::run),
        attributes: BuiltinAttributes::SPECIAL_ASSIGNMENT,
    }, // 27
    BuiltinSpec {
        id: BuiltinId::Return,
        name: b"return",
        handler: BuiltinHandler::Standard(r#return::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 28
    BuiltinSpec {
        id: BuiltinId::Set,
        name: b"set",
        handler: BuiltinHandler::Standard(set::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 29
    BuiltinSpec {
        id: BuiltinId::Shift,
        name: b"shift",
        handler: BuiltinHandler::Standard(shift::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 30
    BuiltinSpec {
        id: BuiltinId::Source,
        name: b"source",
        handler: BuiltinHandler::Standard(dot::run_source),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 31
    BuiltinSpec {
        id: BuiltinId::Test,
        name: b"test",
        handler: BuiltinHandler::Standard(test::run),
        attributes: BuiltinAttributes::NONE,
    }, // 32
    BuiltinSpec {
        id: BuiltinId::Times,
        name: b"times",
        handler: BuiltinHandler::Standard(times::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 33
    BuiltinSpec {
        id: BuiltinId::Trap,
        name: b"trap",
        handler: BuiltinHandler::Standard(trap::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 34
    BuiltinSpec {
        id: BuiltinId::True,
        name: b"true",
        handler: BuiltinHandler::Standard(r#true::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 35
    BuiltinSpec {
        id: BuiltinId::Type,
        name: b"type",
        handler: BuiltinHandler::Standard(r#type::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 36
    BuiltinSpec {
        id: BuiltinId::Ulimit,
        name: b"ulimit",
        handler: BuiltinHandler::Standard(ulimit::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 37
    BuiltinSpec {
        id: BuiltinId::Umask,
        name: b"umask",
        handler: BuiltinHandler::Standard(umask::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 38
    BuiltinSpec {
        id: BuiltinId::Unalias,
        name: b"unalias",
        handler: BuiltinHandler::Standard(unalias::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 39
    BuiltinSpec {
        id: BuiltinId::Unset,
        name: b"unset",
        handler: BuiltinHandler::Standard(unset::run),
        attributes: BuiltinAttributes::SPECIAL,
    }, // 40
    BuiltinSpec {
        id: BuiltinId::Wait,
        name: b"wait",
        handler: BuiltinHandler::Standard(wait::run),
        attributes: BuiltinAttributes::REGULAR,
    }, // 41
];

/// Bash-only built-ins, searched before the baseline table only while the
/// current shell has Bash Compatibility Mode enabled. Keeping a separate
/// sorted table prevents profile-only names from leaking into default mode
/// and permits a future Bash implementation to override a baseline entry.
pub(crate) static BASH_BUILTINS: &[BuiltinSpec] = &[
    BuiltinSpec {
        id: BuiltinId::Builtin,
        name: b"builtin",
        handler: BuiltinHandler::Standard(builtin::run),
        attributes: BuiltinAttributes::SPECIAL,
    },
    BuiltinSpec {
        id: BuiltinId::Caller,
        name: b"caller",
        handler: BuiltinHandler::Standard(caller::run),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Declare,
        name: b"declare",
        handler: BuiltinHandler::Standard(declare::run),
        attributes: BuiltinAttributes::REGULAR_ASSIGNMENT,
    },
    BuiltinSpec {
        id: BuiltinId::Dirs,
        name: b"dirs",
        handler: BuiltinHandler::Standard(dirs::run_dirs),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Enable,
        name: b"enable",
        handler: BuiltinHandler::Standard(enable::run),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Help,
        name: b"help",
        handler: BuiltinHandler::Standard(help::run),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Let,
        name: b"let",
        handler: BuiltinHandler::Standard(r#let::run),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Mapfile,
        name: b"mapfile",
        handler: BuiltinHandler::Standard(mapfile::run),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Popd,
        name: b"popd",
        handler: BuiltinHandler::Standard(dirs::run_popd),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Pushd,
        name: b"pushd",
        handler: BuiltinHandler::Standard(dirs::run_pushd),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Mapfile,
        name: b"readarray",
        handler: BuiltinHandler::Standard(mapfile::run),
        attributes: BuiltinAttributes::REGULAR,
    },
    BuiltinSpec {
        id: BuiltinId::Shopt,
        name: b"shopt",
        handler: BuiltinHandler::Standard(shopt::run),
        attributes: BuiltinAttributes::NONE,
    },
    BuiltinSpec {
        id: BuiltinId::Declare,
        name: b"typeset",
        handler: BuiltinHandler::Standard(declare::run),
        attributes: BuiltinAttributes::REGULAR_ASSIGNMENT,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use bstr::BString;

    use crate::expand::ExpandedField;

    #[test]
    fn args_borrow_complete_fields() {
        let fields = vec![
            ExpandedField {
                text: BString::from(&b"echo"[..]),
            },
            ExpandedField {
                text: BString::new(Vec::new()),
            },
            ExpandedField {
                text: BString::from(&b"a b"[..]),
            },
        ];
        let args = args(&fields);
        assert_eq!(
            args,
            vec![BStr::new("echo"), BStr::new(""), BStr::new("a b")]
        );
    }

    /// Every row the table names resolves, which is the check that a
    /// module move did not leave a name pointing at the wrong function.
    // [spec:nsh:req:idiom.builtin-registry/test]
    #[test]
    fn every_row_has_a_typed_handler() {
        for spec in BUILTINS {
            match (spec.id(), spec.handler()) {
                (BuiltinId::Eval, BuiltinHandler::Eval)
                | (BuiltinId::Fc, BuiltinHandler::History)
                | (_, BuiltinHandler::Standard(_)) => {}
                (id, _) => panic!("{id:?} has the wrong handler kind"),
            }
        }
        for spec in BASH_BUILTINS {
            assert!(matches!(spec.handler(), BuiltinHandler::Standard(_)));
        }
    }
}
