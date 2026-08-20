//! Structural checks for parser and expander control flow.

const PARSER: &str = include_str!("../src/parser.rs");
const PARSER_MULTIBYTE: &str = include_str!("../src/parser/multibyte.rs");
const EXPANDER: &str = include_str!("../src/expand.rs");
const EXPANSION_MODES: &str = include_str!("../src/expand/mode.rs");
const EVALUATOR: &str = include_str!("../src/eval.rs");
const REDIRECTIONS: &str = include_str!("../src/redir.rs");
const JOBS: &str = include_str!("../src/jobs.rs");

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
