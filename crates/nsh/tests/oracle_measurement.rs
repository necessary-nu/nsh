//! A check must not be able to pass while measuring nothing.
//!
//! [`spec:nsh:req:oracle.cannot-measure-is-a-failure`] made mechanical.
//! Five of this repository's own checks were found incapable of failing,
//! and between them they took two shapes: a check that returns early
//! because its reference is missing, and a check whose assertions all sit
//! inside an `if let Ok(…)` whose other arm asserts nothing. Both are
//! findable in the source text, which is what makes the rule worth more
//! than prose.
//!
//! # What is not flagged
//!
//! A check that does not apply to the host it is running on is permitted,
//! and the rule asks it to say so statically. In this workspace the
//! static spelling is a `pub const fn … -> bool` in `nsh-platform`: the
//! platform boundary keeps `cfg(target_os …)` out of the shell, so a
//! host fact reaches a test as a call that the compiler has already
//! folded to a constant. This lint reads those declarations and treats a
//! guard made only of one as the static skip it is. Turn such a predicate
//! into a runtime probe and every guard using it starts being reported,
//! which is the distinction the rule turns on.
//!
//! # The opt-out
//!
//! An early return unrelated to a reference is ordinary code. Suppress a
//! report with a line comment anywhere inside the reported check:
//!
//! ```text
//! // oracle-violation: early-return=the loop below asserts on every case
//! ```
//!
//! The grammar is `<code>=<reason>`, the same shape `nplan commit
//! --violation` uses, for the same purpose: the bypass is a recorded
//! decision rather than a silent one. A suppression that suppresses
//! nothing is itself reported, so they cannot outlive their reason.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The tree this lint reads: everything that reports a pass or a fail.
///
/// `fuzz/` is deliberately outside it. A fuzz target returns early on
/// every input it finds uninteresting -- an `Arbitrary` that did not
/// parse, a byte string with a NUL in it -- and that is the fuzzing loop
/// rather than a missing reference. Reading the eleven targets reports
/// twenty-five of those and one real defect, and a lint that needs
/// twenty-five suppressions on its first day does not survive to catch
/// the twenty-sixth. What the fuzz side has instead is
/// `fuzz/fuzz_targets/support.rs`, where obtaining the oracle panics, so
/// no target can reach a comparison without one.
const TREES: &[&str] = &["crates"];

/// Where the host facts live. A `pub const fn` here is a compile-time
/// constant at every call site, so a guard made of one is static.
const PLATFORM_FACTS: &str = "crates/nsh-platform/src";

/// The two shapes, and the codes their suppressions carry.
const EARLY_RETURN: &str = "early-return";
const UNMEASURED_BRANCH: &str = "unmeasured-branch";

/// Ways a check says it measured something. `unwrap` and `expect` are
/// here because they fail the run, which is the property that matters.
const ASSERTIONS: &[&str] = &[
    "assert!",
    "assert_eq!",
    "assert_ne!",
    "assert_matches!",
    "debug_assert",
    "panic!",
    "unreachable!",
    "todo!",
    "unimplemented!",
    ".unwrap(",
    ".expect(",
    ".unwrap_err(",
    ".expect_err(",
];

/// A run of source with comments and literal contents blanked out, so
/// that a brace in a string is not structure and `// return` is not code.
///
/// Comments blank to spaces and literals to `~`, and the difference
/// carries weight: a suppression is read out of the raw text at the same
/// offset, and the two spellings are what tell a real comment from a
/// sample of one quoted inside this very file. Byte offsets and line
/// numbers are preserved throughout.
fn masked(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let (blank_to, filler) = match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'/') => (
                bytes[at..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |end| at + end),
                b' ',
            ),
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                let mut depth = 1usize;
                let mut end = at + 2;
                while end + 1 < bytes.len() && depth > 0 {
                    match &bytes[end..end + 2] {
                        b"/*" => {
                            depth += 1;
                            end += 2;
                        }
                        b"*/" => {
                            depth -= 1;
                            end += 2;
                        }
                        _ => end += 1,
                    }
                }
                (end, b' ')
            }
            b'"' => (string_end(bytes, at), b'~'),
            b'\'' => match character_end(bytes, at) {
                Some(end) => (end, b'~'),
                None => {
                    at += 1;
                    continue;
                }
            },
            _ => {
                at += 1;
                continue;
            }
        };
        for byte in &mut out[at..blank_to.min(bytes.len())] {
            if *byte != b'\n' {
                *byte = filler;
            }
        }
        at = blank_to.max(at + 1);
    }
    out
}

/// The byte after the quote closing the literal opening at `quote`,
/// reading the `r###` that may precede it to know how it closes.
fn string_end(bytes: &[u8], quote: usize) -> usize {
    let hashes = bytes[..quote]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'#')
        .count();
    let raw = quote > hashes && bytes.get(quote - hashes - 1) == Some(&b'r');
    let mut at = quote + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' if !raw => at += 2,
            b'"' if !raw => return at + 1,
            b'"' if bytes[at + 1..].iter().take_while(|b| **b == b'#').count() >= hashes => {
                return at + 1 + hashes;
            }
            _ => at += 1,
        }
    }
    bytes.len()
}

/// The byte after a character literal opening at `quote`, or `None` when
/// the quote opens a lifetime instead.
fn character_end(bytes: &[u8], quote: usize) -> Option<usize> {
    let mut at = quote + 1;
    if bytes.get(at) == Some(&b'\\') {
        at += 1;
        at += match bytes.get(at)? {
            b'x' => 3,
            b'u' => bytes[at..].iter().position(|byte| *byte == b'}')? + 1,
            _ => 1,
        };
    } else {
        at += match bytes.get(at)?.leading_ones() {
            0 => 1,
            width => width as usize,
        };
    }
    (bytes.get(at) == Some(&b'\'')).then_some(at + 1)
}

/// The `{` opening the innermost block still open at `at`.
///
/// Counted rather than searched: the nearest `{` behind a statement is
/// often one that has already closed, and taking it would read a
/// finished `if` as the guard standing over the statement.
fn block_opening(masked: &[u8], body: usize, at: usize) -> usize {
    let mut depth = 0usize;
    for (offset, byte) in masked[body..at].iter().enumerate().rev() {
        match byte {
            b'}' => depth += 1,
            b'{' if depth == 0 => return body + offset,
            b'{' => depth -= 1,
            _ => {}
        }
    }
    body
}

/// The byte after the `}` closing the block that opens at `open`.
fn block_end(masked: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (offset, byte) in masked[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return open + offset + 1;
                }
            }
            _ => {}
        }
    }
    masked.len()
}

/// Every `*.rs` below `tree`, sorted, so a report reads the same twice.
fn rust_sources(tree: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(tree) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("source entry is readable").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// The names of the compile-time host facts, read out of `nsh-platform`
/// rather than listed here, so the list cannot drift from the truth.
fn platform_facts(workspace: &Path) -> Vec<String> {
    let mut sources = Vec::new();
    rust_sources(&workspace.join(PLATFORM_FACTS), &mut sources);
    let mut names = Vec::new();
    for path in sources {
        let text = std::fs::read_to_string(&path).expect("platform source is UTF-8");
        for declaration in String::from_utf8_lossy(&masked(&text))
            .split("pub const fn ")
            .skip(1)
        {
            let Some((signature, _)) = declaration.split_once('{') else {
                continue;
            };
            if !signature.contains("-> bool") {
                continue;
            }
            let name: String = signature
                .chars()
                .take_while(|character| character.is_alphanumeric() || *character == '_')
                .collect();
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    names
}

/// A check the lint reads, named for the report, spanning from its
/// first attribute to its closing brace.
struct Check {
    name: String,
    start: usize,
    body: usize,
    end: usize,
}

/// Every `#[test]` function in one masked source.
fn checks(masked: &str) -> Vec<Check> {
    let mut found = Vec::new();
    for (offset, _) in masked.match_indices("#[test]") {
        let Some(head) = masked[offset..].find("fn ").map(|at| offset + at) else {
            continue;
        };
        let Some(body) = masked[head..].find('{').map(|at| head + at) else {
            continue;
        };
        let name: String = masked[head + "fn ".len()..body]
            .trim_start()
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        found.push(Check {
            name,
            start: offset,
            body,
            end: block_end(masked.as_bytes(), body),
        });
    }
    found
}

/// The condition standing between the start of `block` and whatever
/// precedes it: an `if …`, a `let … else`, or a `match` arm's pattern.
fn guard_of(masked: &str, block: usize) -> String {
    let head = masked[..block]
        .rfind([';', '{', '}'])
        .map_or(0, |at| at + 1);
    masked[head..block].trim().to_owned()
}

/// Whether `guard` is the static skip the rule permits: a condition made
/// of nothing but one compile-time host fact.
fn is_static(guard: &str, facts: &[String]) -> bool {
    let condition = guard
        .trim_start_matches("if")
        .trim()
        .trim_start_matches('!')
        .trim();
    let Some(call) = condition.strip_suffix("()") else {
        return false;
    };
    let name = call.rsplit("::").next().unwrap_or(call).trim();
    facts.iter().any(|fact| fact == name)
}

/// The suppressions written inside one check, as `(code, reason, line)`.
///
/// The span starts at the check's own line and walks back over the
/// comments and attributes above it, so a suppression reads naturally
/// either beside the return it excuses or above the check it belongs to.
fn suppressions(raw: &str, masked: &str, check: &Check) -> Vec<(String, String, usize)> {
    let mut head = raw[..check.start].rfind('\n').map_or(0, |at| at + 1);
    while head > 0 {
        let previous = raw[..head - 1].rfind('\n').map_or(0, |at| at + 1);
        let text = raw[previous..head].trim_start();
        if !text.starts_with("//") && !text.starts_with("#[") {
            break;
        }
        head = previous;
    }
    let mut found = Vec::new();
    for (offset, _) in raw[head..check.end].match_indices("// oracle-violation:") {
        let at = head + offset;
        /* A comment blanks to spaces and a literal to `~`, so this is
         * how a sample suppression quoted in a string stays a sample. */
        if masked.as_bytes().get(at) != Some(&b' ') {
            continue;
        }
        let line = raw[at..].lines().next().unwrap_or_default();
        let (code, reason) = line
            .trim_start_matches("// oracle-violation:")
            .trim()
            .split_once('=')
            .unwrap_or_default();
        found.push((
            code.trim().to_owned(),
            reason.trim().to_owned(),
            raw[..at].matches('\n').count() + 1,
        ));
    }
    found
}

/// Every position in `span` at which the check says it measured something.
fn assertions(span: &str) -> Vec<usize> {
    let mut found = Vec::new();
    for marker in ASSERTIONS {
        found.extend(span.match_indices(marker).map(|(at, _)| at));
    }
    found
}

/// The two forbidden shapes in one check, as `(code, line, detail)`.
fn shapes(masked: &str, check: &Check, facts: &[String]) -> Vec<(&'static str, usize, String)> {
    let mut found = Vec::new();
    let body = &masked[check.body..check.end];
    for (offset, _) in body.match_indices("return") {
        let before = body[..offset].chars().next_back();
        if before.is_some_and(|character| character.is_alphanumeric() || character == '_') {
            continue;
        }
        let tail = body[offset + "return".len()..].trim_start();
        /* Only a return that ends the check *reporting success* is the
         * shape. `return None` inside a closure is ordinary control
         * flow and says nothing about whether the check measured. */
        if !tail.starts_with(';') && !tail.starts_with("Ok(())") {
            continue;
        }
        let block = block_opening(masked.as_bytes(), check.body, check.body + offset);
        let guard = guard_of(masked, block);
        if is_static(&guard, facts) {
            continue;
        }
        found.push((
            EARLY_RETURN,
            masked[..check.body + offset].matches('\n').count() + 1,
            format!("returns without measuring, guarded by `{guard}`"),
        ));
    }

    for opening in ["if let Ok(", "if let Some("] {
        for (offset, _) in body.match_indices(opening) {
            let Some(block) = body[offset..].find('{').map(|at| offset + at) else {
                continue;
            };
            let close = block_end(body.as_bytes(), block);
            if body[close..].trim_start().starts_with("else") {
                continue;
            }
            let inside = assertions(&body[block..close]);
            let outside = assertions(&body[..block]).len() + assertions(&body[close..]).len();
            if !inside.is_empty() && outside == 0 {
                found.push((
                    UNMEASURED_BRANCH,
                    masked[..check.body + offset].matches('\n').count() + 1,
                    format!("every assertion sits inside `{opening}…)`, which may not be taken"),
                ));
            }
        }
    }
    found
}

/// One file's reports, and the suppressions that were spent doing it.
fn report(path: &str, raw: &str, facts: &[String]) -> Vec<String> {
    let masked = String::from_utf8_lossy(&masked(raw)).into_owned();
    let mut lines = Vec::new();
    for check in checks(&masked) {
        let found = shapes(&masked, &check, facts);
        let allowed = suppressions(raw, &masked, &check);
        for (code, line, detail) in &found {
            if allowed.iter().any(|(spelling, reason, _)| {
                spelling == code && !reason.is_empty() && reason.len() >= 12
            }) {
                continue;
            }
            let mut text = String::new();
            write!(
                text,
                "{path}:{line}: {} in `{}`: {detail}",
                code, check.name
            )
            .expect("format a report");
            lines.push(text);
        }
        for (code, reason, line) in allowed {
            if !matches!(code.as_str(), EARLY_RETURN | UNMEASURED_BRANCH) {
                lines.push(format!(
                    "{path}:{line}: `{code}` is not a shape this lint reports"
                ));
            } else if reason.len() < 12 {
                lines.push(format!(
                    "{path}:{line}: `{code}=` needs a reason, not `{reason}`"
                ));
            } else if !found.iter().any(|(spelling, _, _)| *spelling == code) {
                lines.push(format!(
                    "{path}:{line}: `{code}` suppresses nothing here; delete it"
                ));
            }
        }
    }
    lines
}

/// The whole corpus, reported at once so a run says everything it found.
fn sweep(workspace: &Path) -> Vec<String> {
    let facts = platform_facts(workspace);
    assert!(
        facts.len() > 4,
        "no compile-time host facts found under {PLATFORM_FACTS}; the static-skip \
         exemption would silently report every host-gated check"
    );
    let mut sources = Vec::new();
    for tree in TREES {
        rust_sources(&workspace.join(tree), &mut sources);
    }
    sources.sort();
    let mut lines = Vec::new();
    for path in sources {
        let raw = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let relative = path
            .strip_prefix(workspace)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        lines.extend(report(&relative, &raw, &facts));
    }
    lines
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn no_check_can_pass_without_measuring() {
    let reports = sweep(&workspace_root());
    assert!(
        reports.is_empty(),
        "A check that cannot reach its reference has measured nothing, and \
         \"could not measure\" is a failure and never a pass \
         ([spec:nsh:req:oracle.cannot-measure-is-a-failure]).\n\n{}\n\n\
         If the report is wrong -- an early return with nothing to do with a \
         reference -- record why, inside the check:\n\
         \x20   // oracle-violation: {EARLY_RETURN}=<reason, at least a dozen characters>\n\
         A check that genuinely does not apply to this host says so statically, \
         with a `pub const fn` host fact from nsh-platform, `cfg`, or `#[ignore]`.\n\
         The lint itself is crates/nsh/tests/oracle_measurement.rs.",
        reports.join("\n")
    );
}

/// The lint, asked to fail.
///
/// A lint that has never been seen to report is the very thing the rule
/// forbids, so the shapes are kept here as text and the reader is run
/// over them. Written as one string per shape rather than as real code
/// because a real violating test in this crate would be found by the
/// sweep above, which is the point.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_forbidden_shapes_are_reported() {
    let facts = vec!["can_unlink_current_directory".to_owned()];
    let guarded_return = "#[test]\nfn t() {\n    if !fixture_exists() {\n        return;\n    }\n    assert!(true);\n}\n";
    let let_else = "#[test]\nfn t() {\n    let Some(reference) = reference_bash() else {\n        return;\n    };\n    assert!(reference.exists());\n}\n";
    let nested = "#[test]\nfn t() {\n    if let Ok(reference) = spawn() {\n        assert_eq!(reference, 1);\n    }\n}\n";
    let static_skip = "#[test]\nfn t() {\n    if !can_unlink_current_directory() {\n        return;\n    }\n    assert!(true);\n}\n";
    let in_a_literal = "#[test]\nfn t() {\n    let script = \"if x; then return; fi\";\n    assert!(!script.is_empty());\n}\n";
    let suppressed = "#[test]\n// oracle-violation: early-return=the loop below asserts on every case\nfn t() {\n    if !fixture_exists() {\n        return;\n    }\n    assert!(true);\n}\n";
    let thin = "#[test]\n// oracle-violation: early-return=no\nfn t() {\n    if !fixture_exists() {\n        return;\n    }\n}\n";
    let stale = "#[test]\n// oracle-violation: early-return=this check has no early return at all\nfn t() {\n    assert!(true);\n}\n";

    /* The guard is the block standing over the return, not the nearest
     * `{` behind it: a finished `if` on a host fact must not be read as
     * excusing a runtime skip further down. */
    let after_a_closed_block = "#[test]\nfn t() {\n    if can_unlink_current_directory() {\n        prepare();\n    }\n    if !fixture_exists() {\n        return;\n    }\n    assert!(true);\n}\n";

    for (source, expected) in [
        (guarded_return, EARLY_RETURN),
        (let_else, EARLY_RETURN),
        (nested, UNMEASURED_BRANCH),
        (after_a_closed_block, EARLY_RETURN),
    ] {
        let reports = report("sample.rs", source, &facts);
        assert_eq!(
            reports.len(),
            1,
            "expected one {expected} report, got {reports:?}"
        );
        assert!(reports[0].contains(expected), "{reports:?}");
    }

    for source in [static_skip, in_a_literal, suppressed] {
        assert!(
            report("sample.rs", source, &facts).is_empty(),
            "reported something it should not have: {:?}",
            report("sample.rs", source, &facts)
        );
    }

    assert!(
        report("sample.rs", thin, &facts)
            .iter()
            .any(|line| line.contains("needs a reason")),
        "a suppression without a reason was accepted"
    );
    assert!(
        report("sample.rs", stale, &facts)
            .iter()
            .any(|line| line.contains("suppresses nothing")),
        "a suppression that suppresses nothing was kept"
    );
}
