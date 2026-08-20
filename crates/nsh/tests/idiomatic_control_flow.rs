//! Structural checks for parser and expander control flow.

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
const SHELL_MAIN: &str = include_str!("../src/shellmain.rs");

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
    for (name, source) in [("read", BUILTIN_READ), ("startup", SHELL_MAIN)] {
        assert!(
            !source.contains("let mut pc"),
            "{name} has a program counter"
        );
        assert!(!source.contains("const L_"), "{name} has translated labels");
    }
    assert!(SHELL_MAIN.contains("enum StartupPhase"));
}

// [spec:nsh:def:idiom.job-control-model/test]
#[test]
fn typed_job_control_model() {
    for required in [
        "struct JobId",
        "enum JobState",
        "pid: ProcessId",
        "status: Option<ChildStatus>",
        "prev_job: Option<JobId>",
        "fn transition_to",
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
        "prev_job: Option<usize>",
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
    assert!(OPTIONS.contains("explicit: OptionSet"));

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
