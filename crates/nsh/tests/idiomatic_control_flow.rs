//! Structural checks for parser and expander control flow.

use std::path::{Path, PathBuf};

const PARSER: &str = include_str!("../src/parser.rs");
const PARSER_MULTIBYTE: &str = include_str!("../src/parser/multibyte.rs");
const EXPANDER: &str = include_str!("../src/expand.rs");
const EXPANSION_MODES: &str = include_str!("../src/expand/mode.rs");
const EVALUATOR: &str = include_str!("../src/eval.rs");
const REDIRECTIONS: &str = include_str!("../src/redir.rs");
const JOBS: &str = include_str!("../src/jobs.rs");
const JOB_MODEL: &str = include_str!("../src/jobs/model.rs");
const OPTIONS: &str = include_str!("../src/options.rs");
const OPTION_MODEL: &str = include_str!("../src/options/model.rs");
const BUILTIN_READ: &str = include_str!("../src/builtins/read.rs");
const BUILTIN_BREAK: &str = include_str!("../src/builtins/break.rs");
const BUILTIN_RETURN: &str = include_str!("../src/builtins/return.rs");
const ARITHMETIC: &str = include_str!("../src/arithmetic.rs");
const PATTERN: &str = include_str!("../src/pattern.rs");
const RUNTIME: &str = include_str!("../src/runtime.rs");
const EDITOR: &str = include_str!("../src/editor/mod.rs");
const ALIASES: &str = include_str!("../src/alias.rs");
const ERRORS: &str = include_str!("../src/error.rs");
const INPUT: &str = include_str!("../src/input.rs");
const MAIL: &str = include_str!("../src/mail.rs");
const EXECUTION: &str = include_str!("../src/exec.rs");
const VARIABLES: &str = include_str!("../src/var.rs");
const ULIMIT: &str = include_str!("../src/builtins/ulimit.rs");
const OUTPUT: &str = include_str!("../src/output.rs");
const BUILTINS: &str = include_str!("../src/builtins/mod.rs");
const LIBRARY: &str = include_str!("../src/lib.rs");
const CLI: &str = include_str!("../../nsh-cli/src/main.rs");
const CLI_INVOCATION: &str = include_str!("../../nsh-cli/src/invocation.rs");

fn rust_sources_below(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("source entry is readable").path();
        if path.is_dir() {
            rust_sources_below(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn character_literal_end(bytes: &[u8], quote: usize) -> Option<usize> {
    let mut cursor = quote + 1;
    let first = *bytes.get(cursor)?;
    if first == b'\\' {
        cursor += 1;
        match *bytes.get(cursor)? {
            b'x' => cursor += 3,
            b'u' => {
                cursor += bytes[cursor..].iter().position(|byte| *byte == b'}')? + 1;
            }
            _ => cursor += 1,
        }
    } else {
        cursor += match first.leading_ones() {
            0 => 1,
            width => width as usize,
        };
    }
    (bytes.get(cursor) == Some(&b'\'')).then_some(cursor + 1)
}

fn contains_c_literal(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"//") {
            at += bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - at);
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            let mut depth = 1usize;
            at += 2;
            while at < bytes.len() && depth != 0 {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            continue;
        }

        let character_quote = if bytes[at] == b'\'' {
            Some(at)
        } else if bytes[at..].starts_with(b"b'") {
            Some(at + 1)
        } else {
            None
        };
        if let Some(end) = character_quote.and_then(|quote| character_literal_end(bytes, quote)) {
            at = end;
            continue;
        }

        let boundary = at == 0 || !bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'_';
        if boundary && bytes[at] == b'c' {
            let mut quote = at + 1;
            if bytes.get(quote) == Some(&b'r') {
                quote += 1;
                while bytes.get(quote) == Some(&b'#') {
                    quote += 1;
                }
            }
            if bytes.get(quote) == Some(&b'"') {
                return true;
            }
        }

        let mut quote = at;
        if matches!(bytes.get(quote), Some(b'b')) {
            quote += 1;
        }
        let raw = if bytes.get(quote) == Some(&b'r') {
            quote += 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            true
        } else {
            false
        };
        if bytes.get(quote) == Some(&b'"') {
            let hashes = quote.saturating_sub(at + usize::from(bytes[at] == b'b') + 1);
            quote += 1;
            loop {
                let Some(relative) = bytes[quote..].iter().position(|byte| *byte == b'"') else {
                    return false;
                };
                quote += relative + 1;
                if !raw {
                    let backslashes = bytes[..quote - 1]
                        .iter()
                        .rev()
                        .take_while(|byte| **byte == b'\\')
                        .count();
                    if backslashes % 2 != 0 {
                        continue;
                    }
                    at = quote;
                    break;
                }
                if bytes
                    .get(quote..quote + hashes)
                    .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                {
                    at = quote + hashes;
                    break;
                }
            }
            continue;
        }
        at += 1;
    }
    false
}

// [spec:nsh:req:idiom.no-port-fossils/test]
#[test]
fn port_fossils_are_absent() {
    let mut sources = Vec::new();
    rust_sources_below(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources.sort();

    let forbidden = [
        "#[cfg(any())]",
        "#if ",
        "#ifdef",
        "#ifndef",
        "TRACE((",
        "FNMATCH_IS_ENABLED",
        "GLOB_IS_ENABLED",
        "IS_DEFINED_SMALL",
        "USE_MEMFD_CREATE",
        "HAVE_F_DUPFD_CLOEXEC",
        "HAVE_TRADITIONAL_FACCESSAT",
        "pub const JOBS",
        "pub const BSD",
        "pub const DEBUG",
        "pub type pointer",
        "fn likely(",
        "fn unlikely(",
        "fn etext(",
        "fn getcmdentry(",
        "fn onsigchild(",
    ];

    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        for fossil in forbidden {
            assert!(
                !source.contains(fossil),
                "{} retains port fossil {fossil:?}",
                path.display()
            );
        }
    }
}

// [spec:nsh:req:idiom.narrow-shell-context/test]
#[test]
fn subsystem_helpers_use_narrow_state() {
    for required in [
        "struct Diagnostics<'a>",
        "impl Diagnostics<'_>",
        "fn diagnostics(&mut self) -> Diagnostics<'_>",
        "fn run_with<S, T>",
        "fn poll_interrupt(context: InterruptContext)",
    ] {
        assert!(ERRORS.contains(required), "missing {required}");
    }

    for forbidden in [
        "fn clear_interrupt_deferral(sh:",
        "fn poll_interrupt(sh:",
        "pub fn cur_pf(sh:",
        "pub fn pf_at(sh:",
        "pub fn take_alias_boundary(sh:",
        "pub fn clear_alias_boundary(sh:",
    ] {
        assert!(
            !ERRORS.contains(forbidden) && !INPUT.contains(forbidden),
            "low-level helper retains universal shell access: {forbidden}"
        );
    }

    for (source, required) in [
        (ALIASES, "impl AliasTable"),
        (INPUT, "pub(crate) fn cur_pf(input: &mut InputStack)"),
        (MAIL, "impl MailState"),
    ] {
        assert!(source.contains(required), "missing {required}");
    }
}

// [spec:nsh:req:idiom.output-results/test]
#[test]
fn output_failures_are_returned() {
    for required in [
        "impl Write for Output",
        "pub(crate) fn flushall(&mut self) -> io::Result<()> {",
        "self.stdout.flush()",
    ] {
        assert!(OUTPUT.contains(required), "missing {required}");
    }

    for forbidden in [
        "OUTPUT_ERR",
        "pub flags:",
        "fn outerr(",
        "fn remember_error",
        "-> c_int {\n    if nsh_platform::write_all",
        "pub fn outmem(",
        "pub fn xwrite(",
    ] {
        assert!(
            !OUTPUT.contains(forbidden),
            "output retains error side channel {forbidden:?}"
        );
    }
}

// [spec:nsh:req:idiom.no-ignored-results/test]
#[test]
fn fallible_results_are_explicit() {
    let mut sources = Vec::new();
    rust_sources_below(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources.sort();

    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        for discarded in ["let _ =", "#[allow(unused_must_use)]"] {
            assert!(
                !source.contains(discarded),
                "{} discards a fallible result with {discarded:?}",
                path.display()
            );
        }
    }

    for required in [
        "fn command_output_error",
        "fn write_output(",
        "fn write_output_fmt(",
        "fn flush_output(",
        "result.map_err(|error| self.command_output_error(error))",
    ] {
        assert!(OUTPUT.contains(required), "missing {required}");
    }
    assert!(
        !JOBS.contains("unwrap_or(ChildStatus::Exited(0))"),
        "missing child status is still hidden as numeric success"
    );
}

// [spec:nsh:req:idiom.no-artificial-limits/test]
#[test]
fn dynamic_values_are_not_clamped() {
    for (name, source, forbidden) in [
        ("jobs", JOBS, "append_ascii"),
        ("jobs", JOBS, "name.len().min(32)"),
        ("parser", PARSER, "message.truncate(63)"),
        ("mail", MAIL, "MAXMBOXES"),
        ("mail", MAIL, ".take(MAXMBOXES)"),
    ] {
        assert!(
            !source.contains(forbidden),
            "{name} retains artificial limit {forbidden:?}"
        );
    }
    assert!(MAIL.contains("mailtime: Vec<i64>"));
}

// [spec:nsh:req:idiom.builtin-registry/test]
#[test]
fn builtin_registry_is_fully_typed() {
    for required in [
        "enum BuiltinId",
        "struct BuiltinAttributes",
        "enum BuiltinHandler",
        "Standard(Builtin)",
        "name: &'static [u8]",
        "static BUILTINS: &[BuiltinSpec]",
    ] {
        assert!(BUILTINS.contains(required), "missing {required}");
    }

    for forbidden in [
        "CStr",
        "c_uint",
        "name: c\"",
        "BUILTIN_",
        "NUMBUILTINS",
        "Option<Builtin>",
        "struct builtincmd",
        "static builtincmd",
        ".flags",
    ] {
        assert!(
            !BUILTINS.contains(forbidden),
            "builtin registry retains C representation {forbidden:?}"
        );
    }
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
#[test]
fn shell_entrypoint_uses_public_runtime() {
    assert!(LIBRARY.contains("pub(crate) mod runtime;"));
    assert!(RUNTIME.contains("pub(crate) fn run("));
    assert!(RUNTIME.contains("startup: &Startup"));
    assert!(CLI.contains("nsh::Shell::builder()"));
    assert!(CLI.contains("shell.run_to_completion(startup)"));
    assert!(CLI_INVOCATION.contains("fn parse("));

    for forbidden in [
        "pub mod shellmain;",
        "pub fn main_fn(",
        "fn procargs(",
        "shellmain::main_fn",
    ] {
        assert!(
            !LIBRARY.contains(forbidden)
                && !RUNTIME.contains(forbidden)
                && !OPTIONS.contains(forbidden)
                && !CLI.contains(forbidden),
            "startup retains translated public entrypoint {forbidden:?}"
        );
    }
}

// [spec:nsh:req:idiom.module-boundaries/test]
#[test]
fn modules_follow_rust_subsystems() {
    for module in ["arithmetic", "pattern", "runtime", "editor"] {
        assert!(
            LIBRARY.contains(&format!("mod {module};")),
            "missing subsystem module {module}"
        );
    }
    for (source, boundary) in [
        (ARITHMETIC, "struct Parser"),
        (PATTERN, "struct Matcher"),
        (RUNTIME, "enum StartupTask"),
        (EDITOR, "mod state;"),
    ] {
        assert!(source.contains(boundary), "missing boundary {boundary}");
    }

    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for old_file in [
        "arith_yacc.rs",
        "pmatch.rs",
        "shellmain.rs",
        "shell.rs",
        "histedit.rs",
        "linedit.rs",
    ] {
        assert!(
            !source.join(old_file).exists(),
            "compatibility module {old_file} still exists"
        );
    }
}

// [spec:nsh:req:idiom.no-mystring/test]
#[test]
fn mystring_module_is_absent() {
    let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mystring.rs");
    assert!(
        !module.exists(),
        "generic compatibility module still exists"
    );
    assert!(!LIBRARY.contains("mod mystring"));
}

// [spec:nsh:req:idiom.no-c-strings-core/test]
#[test]
fn core_strings_are_length_delimited() {
    let mut sources = Vec::new();
    rust_sources_below(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources.sort();

    let forbidden = [
        "CStr",
        "CString",
        "to_bytes_with_nul",
        "from_bytes_with_nul",
        "as_cbytes",
        "from_cbytes",
        "push(0)",
        "push(b'\\0')",
        "extend_from_slice(b\"\\0\")",
        "last(), Some(&0)",
    ];
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        assert!(
            !contains_c_literal(&source),
            "{} contains a C string literal",
            path.display()
        );
        for framing in forbidden {
            assert!(
                !source.contains(framing),
                "{} retains C-string framing {framing:?}",
                path.display()
            );
        }
    }
}

// [spec:nsh:req:idiom.no-abi-scalars-core/test]
#[test]
fn core_avoids_abi_scalars() {
    let mut sources = Vec::new();
    rust_sources_below(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources.sort();

    let aliases = [
        "c_char",
        "c_schar",
        "c_uchar",
        "c_short",
        "c_ushort",
        "c_int",
        "c_uint",
        "c_long",
        "c_ulong",
        "c_longlong",
        "c_ulonglong",
        "c_float",
        "c_double",
        "c_void",
    ];
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        for alias in aliases {
            let retained = source
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|identifier| identifier == alias);
            assert!(!retained, "{} retains ABI scalar {alias}", path.display());
        }
    }

    for (source, domain_type) in [
        (EXECUTION, "struct CommandSearch"),
        (EXECUTION, "path_index: Option<usize>"),
        (ERRORS, "enum Operation"),
        (ERRORS, "fn int_pending() -> bool"),
        (EXPANDER, "enum VariableExpansion"),
        (MAIL, "changed: bool"),
        (VARIABLES, "push: bool"),
        (ULIMIT, "struct LimitSelection"),
    ] {
        assert!(source.contains(domain_type), "missing {domain_type}");
    }
}

// [spec:nsh:req:idiom.parser-control-flow/test]
#[test]
fn control_flow_is_structured() {
    for (name, source) in [("parser", PARSER), ("expander", EXPANDER)] {
        for forbidden in ["goto", "Lbl::", "let mut pc", "const L_"] {
            assert!(
                !source.contains(forbidden),
                "{name} contains forbidden C-style control marker {forbidden:?}"
            );
        }

        assert!(
            !source.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with('\'') && line.contains(": {")
            }),
            "{name} contains a labelled block instead of structured control flow"
        );
    }
}

// [spec:nsh:req:idiom.operation-modes/test]
#[test]
fn operation_modes_are_typed() {
    for (name, source, old_prefix) in [
        ("evaluation", EVALUATOR, "pub const EV_"),
        ("expansion", EXPANSION_MODES, "pub const EXP_"),
        ("escaping", EXPANSION_MODES, "pub const RMESCAPE_"),
        ("redirection", REDIRECTIONS, "pub const REDIR_"),
        ("job display", JOBS, "pub const SHOW_"),
    ] {
        assert!(
            !source.contains(old_prefix),
            "{name} still declares integer operation flags with {old_prefix:?}"
        );
    }

    for (source, typed_mode) in [
        (EVALUATOR, "struct EvalContext"),
        (EXPANSION_MODES, "struct ExpansionMode"),
        (EXPANSION_MODES, "enum EscapeMode"),
        (REDIRECTIONS, "enum RedirectionMode"),
        (JOBS, "enum JobDisplay"),
        (PARSER_MULTIBYTE, "enum MultibyteMode"),
    ] {
        assert!(source.contains(typed_mode), "missing {typed_mode}");
    }
}

// [spec:nsh:req:idiom.evaluator-control-flow/test]
#[test]
fn evaluator_control_is_carried_by_flow() {
    for forbidden in [
        "evalskip",
        "skipcount",
        "SKIPBREAK",
        "SKIPCONT",
        "SKIPFUNC",
        "SKIPFUNCDEF",
    ] {
        for (name, source) in [
            ("evaluator", EVALUATOR),
            ("break builtin", BUILTIN_BREAK),
            ("return builtin", BUILTIN_RETURN),
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} contains ambient control marker {forbidden:?}"
            );
        }
    }

    for variant in ["Break {", "Continue {", "Return {"] {
        assert!(EVALUATOR.contains(variant), "Flow is missing {variant}");
    }
    for (name, source) in [("read", BUILTIN_READ), ("startup", RUNTIME)] {
        assert!(
            !source.contains("let mut pc"),
            "{name} has a program counter"
        );
        assert!(!source.contains("const L_"), "{name} has translated labels");
    }
    assert!(RUNTIME.contains("enum StartupTask"));
}

// [spec:nsh:req:idiom.jobs-startup-control-flow/test]
#[test]
fn jobs_read_startup_are_structured() {
    for (name, source) in [("jobs", JOBS), ("read", BUILTIN_READ), ("startup", RUNTIME)] {
        for forbidden in ["goto", "at_start", "let mut phase", "StartupPhase"] {
            assert!(
                !source.contains(forbidden),
                "{name} retains translated control marker {forbidden:?}"
            );
        }
        assert!(
            !source.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with('\'') && line.contains(": {")
            }),
            "{name} retains a labelled control block"
        );
    }

    assert!(JOBS.contains("fn lookup_job"));
    assert!(JOBS.contains("fn record_child_status"));
    assert!(BUILTIN_READ.contains("fn read_input_line"));
    assert!(BUILTIN_READ.contains("escaped_region_end.take()"));
    assert!(RUNTIME.contains("fn run_startup_task"));
    assert!(RUNTIME.contains("const fn recovery"));
}

// [spec:nsh:def:idiom.job-control-model/test]
// [spec:nsh:req:idiom.job-storage/test]
#[test]
fn typed_job_control_model() {
    for required in [
        "struct JobId",
        "enum JobState",
        "pid: ProcessId",
        "status: Option<ChildStatus>",
        "slots: Vec<Option<Job>>",
        "order: Vec<JobId>",
        "fn transition_to",
        "fn position_running",
        "fn remove",
    ] {
        assert!(JOB_MODEL.contains(required), "missing {required}");
    }
    assert!(JOBS.contains("enum WaitOutcome"));
    assert!(JOBS.contains("ProcessGroupId"));

    for forbidden in [
        "JOBRUNNING",
        "JOBSTOPPED",
        "JOBDONE",
        "state: u8",
        "sigint: u8",
        "jobctl: u8",
        "waited: u8",
        "used: u8",
        "changed: u8",
        "used: bool",
        "prev_job",
        "enum Link",
        "CUR_RUNNING",
        "CUR_STOPPED",
        "CUR_DELETE",
    ] {
        assert!(
            !JOB_MODEL.contains(forbidden) && !JOBS.contains(forbidden),
            "job control retains legacy representation {forbidden:?}"
        );
    }
}

// [spec:nsh:def:idiom.shell-options/test]
#[test]
fn typed_shell_options() {
    for required in [
        "enum ShellOption",
        "struct OptionSet",
        "struct OptionSpec",
        "const OPTION_SPECS",
    ] {
        assert!(OPTION_MODEL.contains(required), "missing {required}");
    }
    assert!(OPTIONS.contains("state: OptionSet"));
    assert!(CLI_INVOCATION.contains("struct OptionState"));
    assert!(CLI_INVOCATION.contains("let mut explicit = Vec::new()"));

    for forbidden in [
        "flags: [c_char",
        "fn flag(",
        "fn set_flag(",
        "pub const eflag",
        "pub const iflag",
        "optnames",
        "optletters",
        "NOPTS",
    ] {
        assert!(
            !OPTIONS.contains(forbidden) && !OPTION_MODEL.contains(forbidden),
            "shell options retain legacy representation {forbidden:?}"
        );
    }
}
