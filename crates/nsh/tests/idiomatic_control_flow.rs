//! Structural checks for parser and expander control flow.

const PARSER: &str = include_str!("../src/parser.rs");
const EXPANDER: &str = include_str!("../src/expand.rs");

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
