//! The repository's source-shape checks: did we write Rust, or transcribe C?
//!
//! `[spec:nsh:req:idiom.regression-gates]` asks for *repository checks* that
//! fail when a translated C idiom comes back -- a `c_int`, a `CString`, an
//! integer program counter, a labelled block left by a `goto`. None of that
//! has a runtime signature: a shell with `state: u8` behaves exactly like one
//! with an enum, so no run distinguishes them and the property can only be
//! read off the source.
//!
//! These ran under `cargo test` until 2026-09-02, where a red run meant
//! either "the shell is broken" or "you renamed a function". They are a
//! check now, wired into `.config/nplan/config.styx` beside `fmt` and
//! `clippy`, and a red run means one thing.
//!
//! A check answers with what it found: `Vec<String>`, empty when the
//! source shape is right. It does not assert, and nothing here catches a
//! panic. A panic reaching `main` means the checker could not read the
//! repository at all, which is a different thing from a check failing and
//! is reported as itself, with a location.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The one check whose unit is a file rather than a module, and whose
/// subject is how much of a file there is rather than what is in it. It
/// keeps its own module because it is the only check here that reads a
/// checked-in register, and because adding it to this file would have
/// walked `main.rs` into the very cap it exists to give warning of.
mod density;

/// The one check whose subject is not the source at all but the plan the
/// source cites: a comment giving a decision as its reason, for a decision
/// the corpus no longer holds in force. It keeps its own module for the
/// same two reasons `density` does -- it reads a checked-in file rather
/// than a needle carried here as a constant, and `main.rs` is close enough
/// to the density cap that a check's worth of functions would push it over
/// the register mark.
mod citations;

/// A module's whole text: the file, and every file in the directory beside it.
///
/// Every check here is about how a subsystem is *written*, not about which
/// file a declaration lands in, so splitting a module must not change the
/// answer. Reading one file made two checks fail on a commit that moved
/// code and changed nothing, and -- the half that matters -- would have let
/// a forbidden shape stop being seen by moving it one file down.
fn module(relative: &str) -> String {
    let root = workspace_root().join("crates");
    let mut text = std::fs::read_to_string(root.join(format!("{relative}.rs"))).unwrap_or_default();
    let directory = root.join(relative);
    let mut nested = Vec::new();
    if directory.is_dir() {
        rust_sources_below(&directory, &mut nested);
        nested.sort();
    }
    for path in nested {
        text.push('\n');
        text.push_str(&std::fs::read_to_string(path).expect("a module file is readable"));
    }
    assert!(!text.is_empty(), "no source at crates/{relative}.rs");
    text
}

static PARSER: LazyLock<String> = LazyLock::new(|| module("nsh/src/parser"));
static PARSER_MULTIBYTE: LazyLock<String> = LazyLock::new(|| module("nsh/src/parser/multibyte"));
static EXPANDER: LazyLock<String> = LazyLock::new(|| module("nsh/src/expand"));
static EXPANSION_MODES: LazyLock<String> = LazyLock::new(|| module("nsh/src/expand/mode"));
static EVALUATOR: LazyLock<String> = LazyLock::new(|| module("nsh/src/evaluation"));
static REDIRECTIONS: LazyLock<String> = LazyLock::new(|| module("nsh/src/redirection"));
static JOBS: LazyLock<String> = LazyLock::new(|| module("nsh/src/jobs"));
static JOB_MODEL: LazyLock<String> = LazyLock::new(|| module("nsh/src/jobs/model"));
static OPTIONS: LazyLock<String> = LazyLock::new(|| module("nsh/src/options"));
static OPTION_MODEL: LazyLock<String> = LazyLock::new(|| module("nsh/src/options/model"));
static BUILTIN_READ: LazyLock<String> = LazyLock::new(|| module("nsh/src/builtins/read"));
static BUILTIN_BREAK: LazyLock<String> = LazyLock::new(|| module("nsh/src/builtins/break"));
static BUILTIN_RETURN: LazyLock<String> = LazyLock::new(|| module("nsh/src/builtins/return"));
static ARITHMETIC: LazyLock<String> = LazyLock::new(|| module("nsh/src/arithmetic"));
static PATTERN: LazyLock<String> = LazyLock::new(|| module("nsh/src/pattern"));
static RUNTIME: LazyLock<String> = LazyLock::new(|| module("nsh/src/runtime"));
static EDITOR: LazyLock<String> = LazyLock::new(|| module("nsh/src/editor/mod"));
static ALIASES: LazyLock<String> = LazyLock::new(|| module("nsh/src/alias"));
static ERRORS: LazyLock<String> = LazyLock::new(|| module("nsh/src/error"));
static INPUT: LazyLock<String> = LazyLock::new(|| module("nsh/src/input"));
static MAIL: LazyLock<String> = LazyLock::new(|| module("nsh/src/mail"));
static EXECUTION: LazyLock<String> = LazyLock::new(|| module("nsh/src/execution"));
static VARIABLES: LazyLock<String> = LazyLock::new(|| module("nsh/src/variables"));
static ULIMIT: LazyLock<String> = LazyLock::new(|| module("nsh/src/builtins/ulimit"));
static OUTPUT: LazyLock<String> = LazyLock::new(|| module("nsh/src/output"));
static BUILTINS: LazyLock<String> = LazyLock::new(|| module("nsh/src/builtins/mod"));
static LIBRARY: LazyLock<String> = LazyLock::new(|| module("nsh/src/lib"));
static CLI: LazyLock<String> = LazyLock::new(|| module("nsh-cli/src/main"));
static CLI_INVOCATION: LazyLock<String> = LazyLock::new(|| module("nsh-cli/src/invocation"));
/// Host scalar spellings that only exist to match a C ABI.
const ABI_SCALARS: &[&str] = &[
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

/// NUL-framed string types and the conversions that produce them.
const C_STRINGS: &[&str] = &[
    "CStr",
    "CString",
    "to_bytes_with_nul",
    "from_bytes_with_nul",
    "as_cbytes",
    "from_cbytes",
];

/// Descriptor spellings that carry a number instead of an owner.
const RAW_DESCRIPTORS: &[&str] = &[
    "RawFd",
    "AsRawFd",
    "IntoRawFd",
    "FromRawFd",
    "BorrowedFd",
    "OwnedFd",
];

/// Descriptor operations that move a number rather than an owner.
const MANUAL_DESCRIPTOR_CALLS: &[&str] = &["dup2", "dup3", "fcntl", "close_range", "posix_spawn"];

/// The crates that name the host directly, and the crates that must not.
const LOW_LEVEL_CRATES: &[&str] = &["libc", "rustix", "windows_sys", "ntapi", "nshedit_plat"];

/// The only files permitted to name the host's low-level facilities.
///
/// One list, shared by every gate in this file: two allowlists that
/// disagree are worse than one, and a new platform file is meant to be an
/// explicit addition here rather than a silent one.
const PRIVATE_PLATFORM_ALLOWLIST: &[&str] = &[
    "crates/nsh-platform/src/descriptor.rs",
    "crates/nsh-platform/src/descriptor_name.rs",
    "crates/nsh-platform/src/editor_terminal.rs",
    "crates/nsh-platform/src/locale.rs",
    "crates/nsh-platform/src/locale/characters.rs",
    "crates/nsh-platform/src/signal_names.rs",
    "crates/nsh-platform/src/terminal.rs",
    "crates/nsh-platform/src/unix.rs",
    "crates/nsh-platform/src/unix/endpoints.rs",
    "crates/nsh-platform/src/unix/errors.rs",
    "crates/nsh-platform/src/unix/paths.rs",
    "crates/nsh-platform/src/unix/process.rs",
    "crates/nsh-platform/src/unix/signals.rs",
    "crates/nsh-platform/src/unix_facts.rs",
    "crates/nsh-platform/src/windows.rs",
    "crates/nsh-platform/src/windows/broker.rs",
    "crates/nsh-platform/src/windows/children.rs",
    "crates/nsh-platform/src/windows/descriptor.rs",
    "crates/nsh-platform/src/windows/editor_terminal.rs",
    "crates/nsh-platform/src/windows/endpoints.rs",
    "crates/nsh-platform/src/windows/errors.rs",
    "crates/nsh-platform/src/windows/paths.rs",
    "crates/nsh-platform/src/windows/process.rs",
    "crates/nsh-platform/src/windows/signals.rs",
    "crates/nsh-platform/src/windows/spawn.rs",
    "crates/nsh-platform/src/windows/terminal.rs",
];

/// Every source tree that must hold the safe-core line, including the
/// tests: a test that reaches for the host directly is a way around the
/// boundary, not an exception to it.
const SAFE_CORE_TREES: &[&str] = &[
    "crates/nsh/src",
    "crates/nsh/tests",
    "crates/nsh-cli/src",
    "crates/nsh-platform/tests",
];

fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    /* Canonical, so a report names `crates/nsh/src/mail.rs` rather than
     * the `crates/nsh-lint/../../crates/...` the join produces. */
    std::fs::canonicalize(&manifest).unwrap_or(manifest)
}

/// The shell's own source, which is what every sweep below is about.
///
/// These were written as `CARGO_MANIFEST_DIR/src` while the checks lived
/// inside `crates/nsh`, where that was the same directory. Moving the
/// checks to a crate of their own retargeted five of them at the checker's
/// own source, where they found the needles they carry as constants and
/// reported them -- and had the needles not been in the checker, they
/// would have passed while examining nothing.
fn shell_source() -> PathBuf {
    workspace_root().join("crates/nsh/src")
}

fn relative_to_workspace(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// One named file's text, or the finding to make when it is not there.
///
/// Several checks name a particular file -- `regex.rs`, `descriptor.rs`,
/// the two shell manifests -- because the property is about what that file
/// says. A file that has moved is therefore a finding about the source
/// shape, reported beside every other finding rather than ending the run.
fn text_at(workspace: &Path, relative: &str) -> Result<String, String> {
    std::fs::read_to_string(workspace.join(relative))
        .map_err(|error| format!("{relative} is not readable: {error}"))
}

fn rust_sources_in(workspace: &Path, tree: &str) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    rust_sources_below(&workspace.join(tree), &mut sources);
    sources.sort();
    sources
}

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

struct SourceScan {
    identifiers: Vec<String>,
    contains_c_literal: bool,
}

fn string_literal_end(bytes: &[u8], start: usize) -> Option<(usize, bool)> {
    let mut quote = start;
    let c_literal = bytes.get(quote) == Some(&b'c');
    if matches!(bytes.get(quote), Some(b'b' | b'c')) {
        quote += 1;
    }
    let raw = if bytes.get(quote) == Some(&b'r') {
        quote += 1;
        true
    } else {
        false
    };
    let hashes_start = quote;
    if raw {
        while bytes.get(quote) == Some(&b'#') {
            quote += 1;
        }
    }
    if bytes.get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - hashes_start;
    quote += 1;

    loop {
        let relative = bytes[quote..].iter().position(|byte| *byte == b'"')?;
        quote += relative;
        if !raw {
            let backslashes = bytes[..quote]
                .iter()
                .rev()
                .take_while(|byte| **byte == b'\\')
                .count();
            quote += 1;
            if backslashes % 2 == 0 {
                return Some((quote, c_literal));
            }
            continue;
        }

        quote += 1;
        if bytes
            .get(quote..quote + hashes)
            .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
        {
            return Some((quote + hashes, c_literal));
        }
    }
}

fn scan_rust_source(source: &str) -> SourceScan {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut identifiers = Vec::new();
    let mut contains_c_literal = false;
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

        if let Some((end, is_c_literal)) = string_literal_end(bytes, at) {
            let boundary =
                at == 0 || !bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'_';
            contains_c_literal |= boundary && is_c_literal;
            at = end;
            continue;
        }

        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' || bytes[at].is_ascii_digit() {
            let start = at;
            at += 1;
            while bytes
                .get(at)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                at += 1;
            }
            identifiers.push(String::from_utf8(bytes[start..at].to_vec()).unwrap());
            continue;
        }
        at += 1;
    }
    SourceScan {
        identifiers,
        contains_c_literal,
    }
}

fn contains_c_literal(source: &str) -> bool {
    scan_rust_source(source).contains_c_literal
}

// [spec:nsh:req:idiom.no-port-fossils/test]
fn port_fossils_are_absent() -> Vec<String> {
    let mut sources = Vec::new();
    rust_sources_below(&shell_source(), &mut sources);
    sources.sort();

    let forbidden = [
        "#[cfg(any())]",
        "#if ",
        "#ifdef",
        "#ifndef",
        "#define ",
        "HAVE_",
        "NOTREACHED",
        "__attribute__((",
        "TRACE((",
        "FNMATCH_IS_ENABLED",
        "GLOB_IS_ENABLED",
        "IS_DEFINED_SMALL",
        "USE_MEMFD_CREATE",
        "HAVE_F_DUPFD_CLOEXEC",
        "HAVE_TRADITIONAL_FACCESSAT",
        "pub const JOBS.as_str()",
        "pub const BSD",
        "pub const DEBUG",
        "pub type pointer",
        "fn likely(",
        "fn unlikely(",
        "fn etext(",
        "fn getcmdentry(",
        "fn onsigchild(",
    ];

    let mut reported = Vec::new();
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        for fossil in forbidden {
            if source.contains(fossil) {
                reported.push(format!("{} retains port fossil {fossil:?}", path.display()));
            }
        }
    }
    reported
}

// [spec:nsh:req:idiom.strict-lints/test]
fn core_enforces_strict_rust_lints() -> Vec<String> {
    let mut reported = Vec::new();
    for lint in [
        "unsafe_code",
        "dead_code",
        "non_camel_case_types",
        "non_snake_case",
        "non_upper_case_globals",
        "unused_variables",
        "unused_must_use",
        "clippy::correctness",
    ] {
        let directive = format!("#![deny({lint})]");
        if !LIBRARY.contains(&directive) {
            reported.push(format!("missing strict lint {directive}"));
        }
    }

    if LIBRARY.lines().any(|line| line.starts_with("#![allow(")) {
        reported.push("core crate root retains a blanket lint allowance".to_owned());
    }
    reported
}

// [spec:nsh:req:idiom.regression-gates/test]
fn zero_cism_gate_covers_boundary() -> Vec<String> {
    const CONTROL_BYTE_WORDS: &[&str] = &[
        "CTLESC",
        "CTLVAR",
        "CTLENDVAR",
        "CTLBACKQ",
        "CTLARI",
        "CTLENDARI",
        "CTLQUOTEMARK",
        "CTLMBCHAR",
        "EncodedWord",
        "encode_legacy",
        "from_legacy_fragment",
        "LEGACY_ESCAPE",
        "LEGACY_VARIABLE",
        "LEGACY_VARIABLE_END",
        "LEGACY_COMMAND",
        "LEGACY_ARITHMETIC",
        "LEGACY_ARITHMETIC_END",
        "LEGACY_QUOTE",
        "LEGACY_MULTIBYTE",
        "0x81",
        "0x82",
        "0x83",
        "0x84",
        "0x85",
        "0x86",
        "0x87",
        "0x88",
    ];
    let workspace = workspace_root();
    let mut violations = Vec::new();
    for root in ["crates/nsh/src", "crates/nsh-cli/src"] {
        let mut sources = Vec::new();
        rust_sources_below(&workspace.join(root), &mut sources);
        sources.sort();
        for path in sources {
            let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
            let scan = scan_rust_source(&source);
            let identifiers: Vec<&str> = scan.identifiers.iter().map(String::as_str).collect();
            let relative = path.strip_prefix(&workspace).unwrap().display();

            for forbidden in ABI_SCALARS
                .iter()
                .chain(C_STRINGS)
                .chain(CONTROL_BYTE_WORDS)
                .chain(RAW_DESCRIPTORS)
                .chain(["unsafe", "libc"].iter())
            {
                if identifiers.contains(forbidden) {
                    violations.push(format!("{relative} contains forbidden token {forbidden}"));
                }
            }
            if scan.contains_c_literal {
                violations.push(format!("{relative} contains a C string literal"));
            }
            if source
                .lines()
                .any(|line| line.trim_start().starts_with("#![allow"))
            {
                violations.push(format!("{relative} contains a blanket lint allowance"));
            }
            for tokens in scan.identifiers.windows(3) {
                if matches!(tokens, [first, second, third] if first == "let" && second == "mut" && third == "pc")
                {
                    violations.push(format!(
                        "{relative} contains a mutable integer program counter"
                    ));
                }
            }
            for tokens in scan.identifiers.windows(2) {
                if matches!(tokens, [first, second] if first == "enum" && second == "Lbl")
                    || matches!(tokens, [first, second] if first == "const" && second.starts_with("L_"))
                {
                    violations.push(format!("{relative} contains translated labels"));
                }
            }
        }
    }

    let mut platform_sources = Vec::new();
    rust_sources_below(
        &workspace.join("crates/nsh-platform/src"),
        &mut platform_sources,
    );
    for path in platform_sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let scan = scan_rust_source(&source);
        let low_level = scan.identifiers.iter().any(|identifier| {
            ABI_SCALARS
                .iter()
                .chain(C_STRINGS)
                .chain(RAW_DESCRIPTORS)
                .chain(LOW_LEVEL_CRATES)
                .chain(["unsafe"].iter())
                .any(|forbidden| identifier == forbidden)
        }) || scan.contains_c_literal;
        let relative = path
            .strip_prefix(&workspace)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if low_level && !PRIVATE_PLATFORM_ALLOWLIST.contains(&relative.as_ref()) {
            violations.push(format!(
                "{relative} uses low-level host facilities outside the private allowlist"
            ));
        }
    }

    for manifest in ["crates/nsh/Cargo.toml", "crates/nsh-cli/Cargo.toml"] {
        let text = match text_at(&workspace, manifest) {
            Ok(text) => text,
            Err(report) => {
                violations.push(report);
                continue;
            }
        };
        for dependency in ["libc", "rustix", "windows-sys", "ntapi"] {
            if text.lines().any(|line| {
                line.split_once('=')
                    .is_some_and(|(name, _)| name.trim() == dependency)
            }) {
                violations.push(format!(
                    "{manifest} directly depends on low-level crate {dependency}"
                ));
            }
        }
    }

    violations
}

// [spec:nsh:req:idiom.narrow-shell-context/test]
fn subsystem_helpers_use_narrow_state() -> Vec<String> {
    let mut reported = Vec::new();
    for required in [
        "struct Diagnostics<'a>",
        "impl Diagnostics<'_>",
        "fn diagnostics(&mut self) -> Diagnostics<'_>",
        "fn run_with<S, T>",
        "fn poll_interrupt(context: InterruptContext)",
    ] {
        if !ERRORS.contains(required) {
            reported.push(format!("missing {required}"));
        }
    }

    for forbidden in [
        "fn clear_interrupt_deferral(sh:",
        "fn poll_interrupt(sh:",
        "pub fn cur_pf(sh:",
        "pub fn pf_at(sh:",
        "pub fn take_alias_boundary(sh:",
        "pub fn clear_alias_boundary(sh:",
    ] {
        if ERRORS.contains(forbidden) || INPUT.contains(forbidden) {
            reported.push(format!(
                "low-level helper retains universal shell access: {forbidden}"
            ));
        }
    }

    for (source, required) in [
        (ALIASES.as_str(), "impl AliasTable"),
        (
            INPUT.as_str(),
            "pub(crate) fn current_input_frame(input: &mut InputStack)",
        ),
        (MAIL.as_str(), "impl MailState"),
    ] {
        if !source.contains(required) {
            reported.push(format!("missing {required}"));
        }
    }
    reported
}

// [spec:nsh:req:idiom.output-results/test]
fn output_failures_are_returned() -> Vec<String> {
    let mut reported = Vec::new();
    for required in [
        "impl Write for Output",
        "pub(crate) fn flush_all(&mut self) -> io::Result<()> {",
        "self.stdout.flush()",
    ] {
        if !OUTPUT.contains(required) {
            reported.push(format!("missing {required}"));
        }
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
        if OUTPUT.contains(forbidden) {
            reported.push(format!("output retains error side channel {forbidden:?}"));
        }
    }
    reported
}

// [spec:nsh:req:idiom.no-ignored-results/test]
fn fallible_results_are_explicit() -> Vec<String> {
    let mut sources = Vec::new();
    rust_sources_below(&shell_source(), &mut sources);
    sources.sort();

    let mut reported = Vec::new();
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        for discarded in ["let _ =", "#[allow(unused_must_use)]"] {
            if source.contains(discarded) {
                reported.push(format!(
                    "{} discards a fallible result with {discarded:?}",
                    path.display()
                ));
            }
        }
    }

    for required in [
        "fn command_output_error",
        "fn write_output(",
        "fn write_output_fmt(",
        "fn flush_output(",
        "result.map_err(|error| self.command_output_error(error))",
    ] {
        if !OUTPUT.contains(required) {
            reported.push(format!("missing {required}"));
        }
    }
    if JOBS.contains("unwrap_or(ChildStatus::Exited(0))") {
        reported.push("missing child status is still hidden as numeric success".to_owned());
    }
    reported
}

// [spec:nsh:req:idiom.no-artificial-limits/test]
fn dynamic_values_are_not_clamped() -> Vec<String> {
    let mut reported = Vec::new();
    for (name, source, forbidden) in [
        ("jobs", JOBS.as_str(), "append_ascii"),
        ("jobs", JOBS.as_str(), "name.len().min(32)"),
        ("parser", PARSER.as_str(), "message.truncate(63)"),
        ("mail", MAIL.as_str(), "MAXMBOXES"),
        ("mail", MAIL.as_str(), ".take(MAXMBOXES)"),
    ] {
        if source.contains(forbidden) {
            reported.push(format!("{name} retains artificial limit {forbidden:?}"));
        }
    }
    if !MAIL.contains("mailtime: Vec<i64>") {
        reported.push("missing mailtime: Vec<i64>".to_owned());
    }
    reported
}

// [spec:nsh:req:idiom.builtin-registry/test]
fn builtin_registry_is_fully_typed() -> Vec<String> {
    let mut reported = Vec::new();
    for required in [
        "enum BuiltinId",
        "struct BuiltinAttributes",
        "enum BuiltinHandler",
        "Standard(Builtin)",
        "name: &'static [u8]",
        "static BUILTINS: &[BuiltinSpec]",
    ] {
        if !BUILTINS.contains(required) {
            reported.push(format!("missing {required}"));
        }
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
        if BUILTINS.contains(forbidden) {
            reported.push(format!(
                "builtin registry retains C representation {forbidden:?}"
            ));
        }
    }
    reported
}

// [spec:nsh:req:idiom.shell-entrypoint/test]
fn shell_entrypoint_uses_public_runtime() -> Vec<String> {
    let mut reported = Vec::new();
    for (source, required) in [
        (RUNTIME.as_str(), "startup: &Startup"),
        (CLI.as_str(), "nsh::Shell::builder()"),
        (CLI.as_str(), "shell.run_to_completion(startup)"),
    ] {
        if !source.contains(required) {
            reported.push(format!("missing {required}"));
        }
    }

    for forbidden in [
        "pub mod shellmain;",
        "pub fn main_fn(",
        "fn procargs(",
        "shellmain::main_fn",
    ] {
        if LIBRARY.contains(forbidden)
            || RUNTIME.contains(forbidden)
            || OPTIONS.contains(forbidden)
            || CLI.contains(forbidden)
        {
            reported.push(format!(
                "startup retains translated public entrypoint {forbidden:?}"
            ));
        }
    }
    reported
}

// [spec:nsh:req:idiom.module-boundaries/test]
fn modules_follow_rust_subsystems() -> Vec<String> {
    let mut reported = Vec::new();
    for module in ["arithmetic", "pattern", "runtime", "editor"] {
        if !LIBRARY.contains(&format!("mod {module};")) {
            reported.push(format!("missing subsystem module {module}"));
        }
    }
    for (source, boundary) in [
        (ARITHMETIC.as_str(), "struct Parser"),
        (PATTERN.as_str(), "struct Matcher"),
        (RUNTIME.as_str(), "enum StartupTask"),
        (EDITOR.as_str(), "mod state;"),
    ] {
        if !source.contains(boundary) {
            reported.push(format!("missing boundary {boundary}"));
        }
    }

    let source = shell_source();
    for old_file in [
        "arith_yacc.rs",
        "pmatch.rs",
        "shellmain.rs",
        "shell.rs",
        "histedit.rs",
        "linedit.rs",
    ] {
        if source.join(old_file).exists() {
            reported.push(format!("compatibility module {old_file} still exists"));
        }
    }
    reported
}

// [spec:nsh:req:idiom.no-mystring/test]
fn mystring_module_is_absent() -> Vec<String> {
    let mut reported = Vec::new();
    if shell_source().join("mystring.rs").exists() {
        reported.push("generic compatibility module still exists".to_owned());
    }
    if LIBRARY.contains("mod mystring") {
        reported.push("the core crate root still declares mod mystring".to_owned());
    }
    reported
}

// [spec:nsh:req:idiom.no-c-strings-core/test]
fn core_strings_are_length_delimited() -> Vec<String> {
    let mut sources = Vec::new();
    rust_sources_below(&shell_source(), &mut sources);
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
    let mut reported = Vec::new();
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        if contains_c_literal(&source) {
            reported.push(format!("{} contains a C string literal", path.display()));
        }
        for framing in forbidden {
            if source.contains(framing) {
                reported.push(format!(
                    "{} retains C-string framing {framing:?}",
                    path.display()
                ));
            }
        }
    }
    reported
}

// [spec:nsh:req:idiom.no-abi-scalars-core/test]
fn core_avoids_abi_scalars() -> Vec<String> {
    let mut sources = Vec::new();
    rust_sources_below(&shell_source(), &mut sources);
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
    let mut reported = Vec::new();
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        for alias in aliases {
            let retained = source
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|identifier| identifier == alias);
            if retained {
                reported.push(format!("{} retains ABI scalar {alias}", path.display()));
            }
        }
    }

    for (source, domain_type) in [
        (EXECUTION.as_str(), "struct CommandSearch"),
        (EXECUTION.as_str(), "path_index: Option<usize>"),
        (ERRORS.as_str(), "enum Operation"),
        (ERRORS.as_str(), "fn interrupt_pending() -> bool"),
        (EXPANSION_MODES.as_str(), "struct ExpansionMode"),
        (MAIL.as_str(), "changed: bool"),
        (VARIABLES.as_str(), "push: bool"),
        (ULIMIT.as_str(), "struct LimitSelection"),
    ] {
        if !source.contains(domain_type) {
            reported.push(format!("missing {domain_type}"));
        }
    }
    reported
}

// [spec:nsh:req:idiom.parser-control-flow/test]
fn control_flow_is_structured() -> Vec<String> {
    let mut reported = Vec::new();
    for (name, source) in [("parser", PARSER.as_str()), ("expander", EXPANDER.as_str())] {
        for forbidden in ["goto", "Lbl::", "let mut pc", "const L_"] {
            if source.contains(forbidden) {
                reported.push(format!(
                    "{name} contains forbidden C-style control marker {forbidden:?}"
                ));
            }
        }

        if source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with('\'') && line.contains(": {")
        }) {
            reported.push(format!(
                "{name} contains a labelled block instead of structured control flow"
            ));
        }
    }
    reported
}

// [spec:nsh:req:idiom.operation-modes/test]
fn operation_modes_are_typed() -> Vec<String> {
    let mut reported = Vec::new();
    for (name, source, old_prefix) in [
        ("evaluation", EVALUATOR.as_str(), "pub const EV_"),
        ("expansion", EXPANSION_MODES.as_str(), "pub const EXP_"),
        ("escaping", EXPANSION_MODES.as_str(), "pub const RMESCAPE_"),
        ("redirection", REDIRECTIONS.as_str(), "pub const REDIR_"),
        ("job display", JOBS.as_str(), "pub const SHOW_"),
    ] {
        if source.contains(old_prefix) {
            reported.push(format!(
                "{name} still declares integer operation flags with {old_prefix:?}"
            ));
        }
    }

    for (source, typed_mode) in [
        (EVALUATOR.as_str(), "struct EvaluationContext"),
        (EXPANSION_MODES.as_str(), "struct ExpansionMode"),
        (REDIRECTIONS.as_str(), "enum RedirectionMode"),
        (JOBS.as_str(), "enum JobDisplay"),
        (PARSER_MULTIBYTE.as_str(), "enum MultibyteMode"),
    ] {
        if !source.contains(typed_mode) {
            reported.push(format!("missing {typed_mode}"));
        }
    }
    reported
}

// [spec:nsh:req:idiom.evaluator-control-flow/test]
fn evaluator_control_is_carried_by_flow() -> Vec<String> {
    let mut reported = Vec::new();
    for forbidden in [
        "evalskip",
        "skipcount",
        "SKIPBREAK",
        "SKIPCONT",
        "SKIPFUNC",
        "SKIPFUNCDEF",
    ] {
        for (name, source) in [
            ("evaluator", EVALUATOR.as_str()),
            ("break builtin", BUILTIN_BREAK.as_str()),
            ("return builtin", BUILTIN_RETURN.as_str()),
        ] {
            if source.contains(forbidden) {
                reported.push(format!(
                    "{name} contains ambient control marker {forbidden:?}"
                ));
            }
        }
    }

    for variant in ["Break {", "Continue {", "Return {"] {
        if !EVALUATOR.contains(variant) {
            reported.push(format!("Flow is missing {variant}"));
        }
    }
    for (name, source) in [
        ("read", BUILTIN_READ.as_str()),
        ("startup", RUNTIME.as_str()),
    ] {
        if source.contains("let mut pc") {
            reported.push(format!("{name} has a program counter"));
        }
        if source.contains("const L_") {
            reported.push(format!("{name} has translated labels"));
        }
    }
    reported
}

// [spec:nsh:req:idiom.jobs-startup-control-flow/test]
fn jobs_read_startup_are_structured() -> Vec<String> {
    let mut reported = Vec::new();
    for (name, source) in [
        ("jobs", JOBS.as_str()),
        ("read", BUILTIN_READ.as_str()),
        ("startup", RUNTIME.as_str()),
    ] {
        for forbidden in ["goto", "at_start", "let mut phase", "StartupPhase"] {
            if source.contains(forbidden) {
                reported.push(format!(
                    "{name} retains translated control marker {forbidden:?}"
                ));
            }
        }
        if source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with('\'') && line.contains(": {")
        }) {
            reported.push(format!("{name} retains a labelled control block"));
        }
    }

    if !BUILTIN_READ.contains("protected: Vec<bool>") {
        reported.push("missing protected: Vec<bool>".to_owned());
    }
    if !RUNTIME.contains("const fn recovery") {
        reported.push("missing const fn recovery".to_owned());
    }
    reported
}

// [spec:nsh:def:idiom.job-control-model/test]
// [spec:nsh:req:idiom.job-storage/test]
fn typed_job_control_model() -> Vec<String> {
    let mut reported = Vec::new();
    for required in [
        "struct JobId",
        "enum JobState",
        "process_id: ProcessId",
        "status: Option<ChildStatus>",
        "slots: Vec<Option<Job>>",
        "order: Vec<JobId>",
        "fn transition_to",
        "fn position_running",
        "fn remove",
    ] {
        if !JOB_MODEL.contains(required) {
            reported.push(format!("missing {required}"));
        }
    }
    if !JOBS.contains("ProcessGroupId") {
        reported.push("missing ProcessGroupId".to_owned());
    }

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
        if JOB_MODEL.contains(forbidden) || JOBS.contains(forbidden) {
            reported.push(format!(
                "job control retains legacy representation {forbidden:?}"
            ));
        }
    }
    reported
}

// [spec:nsh:def:idiom.shell-options/test]
fn typed_shell_options() -> Vec<String> {
    let mut reported = Vec::new();
    for required in [
        "enum ShellOption",
        "struct OptionSet",
        "struct OptionSpec",
        "const OPTION_SPECS",
    ] {
        if !OPTION_MODEL.contains(required) {
            reported.push(format!("missing {required}"));
        }
    }
    if !OPTIONS.contains("state: OptionSet") {
        reported.push("missing state: OptionSet".to_owned());
    }
    if !CLI_INVOCATION.contains("let mut explicit = Vec::new()") {
        reported.push("missing let mut explicit = Vec::new()".to_owned());
    }

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
        if OPTIONS.contains(forbidden) || OPTION_MODEL.contains(forbidden) {
            reported.push(format!(
                "shell options retain legacy representation {forbidden:?}"
            ));
        }
    }
    reported
}

// ---- the Bash compatibility delta's safe-core gate ------------------

/// Does this line open an `unsafe` expression block?
///
/// `unsafe extern` declaration blocks and `unsafe fn` signatures are
/// excluded: the obligation they carry belongs to their call sites, which
/// are the blocks this looks for.
fn opens_unsafe_block(line: &str) -> bool {
    let Some(at) = line.find("unsafe") else {
        return false;
    };
    let before_is_word = line[..at]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_');
    !before_is_word && line[at + "unsafe".len()..].trim_start().starts_with('{')
}

/// Every source file below one of the safe-core trees, with its scan.
fn scanned_safe_core_sources(workspace: &Path) -> Vec<(String, SourceScan)> {
    let mut scanned = Vec::new();
    for tree in SAFE_CORE_TREES {
        for path in rust_sources_in(workspace, tree) {
            let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
            scanned.push((
                relative_to_workspace(&path, workspace),
                scan_rust_source(&source),
            ));
        }
    }
    scanned
}

/// The compatibility delta stays safe Rust, tests included.
///
/// The rule names five things Bash work may not introduce: `unsafe`,
/// direct `libc`, `RawFd` storage or parameters, and manual `dup2` or
/// `close`. The first four are identifiers, so they are matched as
/// identifiers rather than as substrings -- a comment discussing `unsafe`
/// or a table listing `"RawFd"` is prose, not a bypass. Manual descriptor
/// moves are matched the same way.
// [spec:nsh:req:compat.bash.safe-core/test]
fn the_compatibility_delta_stays_safe() -> Vec<String> {
    let workspace = workspace_root();
    let mut violations = Vec::new();

    for (relative, scan) in scanned_safe_core_sources(&workspace) {
        for forbidden in ABI_SCALARS
            .iter()
            .chain(C_STRINGS)
            .chain(RAW_DESCRIPTORS)
            .chain(MANUAL_DESCRIPTOR_CALLS)
            .chain(LOW_LEVEL_CRATES)
            .chain(["unsafe"].iter())
        {
            if scan.identifiers.iter().any(|name| name == forbidden) {
                violations.push(format!("{relative} names {forbidden}"));
            }
        }
        if scan.contains_c_literal {
            violations.push(format!("{relative} contains a C string literal"));
        }
    }

    violations
}

/// `unsafe` lives in the platform crate, in named files, with a reason.
///
/// The allowlist is exact in both directions: a file that names the host
/// must be on it, and every entry must still exist, so a deleted file
/// cannot leave a permission behind for a later one to inherit.
// [spec:nsh:req:compat.bash.safe-core/test]
fn platform_unsafe_is_named_and_justified() -> Vec<String> {
    let workspace = workspace_root();
    let mut violations = Vec::new();

    for entry in PRIVATE_PLATFORM_ALLOWLIST {
        if !workspace.join(entry).exists() {
            violations.push(format!("{entry} is allowlisted but does not exist"));
        }
    }

    for path in rust_sources_in(&workspace, "crates/nsh-platform/src") {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let relative = relative_to_workspace(&path, &workspace);
        let scan = scan_rust_source(&source);
        let names_host = scan
            .identifiers
            .iter()
            .any(|name| name == "unsafe" || LOW_LEVEL_CRATES.contains(&name.as_str()));
        if names_host && !PRIVATE_PLATFORM_ALLOWLIST.contains(&relative.as_str()) {
            violations.push(format!("{relative} names the host outside the allowlist"));
        }

        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if opens_unsafe_block(line)
                && !lines[index.saturating_sub(8)..index]
                    .iter()
                    .any(|earlier| earlier.contains("SAFETY"))
            {
                violations.push(format!(
                    "{relative}:{} opens an unsafe block with no SAFETY note",
                    index + 1
                ));
            }
        }
    }

    violations
}

/// A descriptor crosses the platform boundary as an owner, never a number.
///
/// `dup2` and `close` on a raw number exist exactly once each in the whole
/// workspace, inside the one transaction that materializes a child's
/// descriptor table after the last fork. Anywhere else they would be a
/// second, unowned lifetime for a descriptor the shell already owns.
// [spec:nsh:req:compat.bash.safe-core/test]
fn descriptors_cross_the_boundary_owned() -> Vec<String> {
    let workspace = workspace_root();
    let mut violations = Vec::new();

    match text_at(&workspace, "crates/nsh-platform/src/descriptor.rs") {
        Err(report) => violations.push(report),
        Ok(descriptor) => {
            for required in [
                "pub(crate) fn number(&self) -> i32",
                "pub(crate) fn borrowed(&self) -> BorrowedFd<'_>",
            ] {
                if !descriptor.contains(required) {
                    violations.push(format!(
                        "the descriptor number stopped being private: {required}"
                    ));
                }
            }
        }
    }

    let mut manual_moves = 0_usize;
    for path in rust_sources_in(&workspace, "crates/nsh-platform/src") {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let relative = relative_to_workspace(&path, &workspace);
        manual_moves += source.matches("libc::dup2(").count();
        manual_moves += source.matches("libc::close(").count();

        for line in source.lines() {
            let line = line.trim_start();
            if !line.starts_with("pub fn") && !line.starts_with("pub unsafe fn") {
                continue;
            }
            for spelling in RAW_DESCRIPTORS {
                if line.contains(spelling) {
                    violations.push(format!("{relative} exposes {spelling} in `{line}`"));
                }
            }
        }
    }
    if manual_moves != 2 {
        violations.push(format!(
            "manual descriptor moves are no longer the two in the materialization \
             transaction: {manual_moves}"
        ));
    }

    match text_at(&workspace, "crates/nsh-platform/src/descriptor_name.rs") {
        Err(report) => violations.push(report),
        Ok(source) => {
            for (function, borrow) in [
                ("descriptor_name", "fd: &Descriptor"),
                ("publish_descriptor_across_exec", "fd: &Descriptor"),
            ] {
                if !source.contains(&format!("pub fn {function}({borrow}")) {
                    violations.push(format!("{function} stopped borrowing its descriptor"));
                }
            }
        }
    }

    /* The terminal entry point that stood beside this one arrived with the
     * interactive surface and left with it; `wait_for_input` stayed because
     * `read -t` is a script-visible builtin. */
    let required = "pub fn wait_for_input(fd: &impl AsDescriptor";
    match text_at(&workspace, "crates/nsh-platform/src/unix_facts.rs") {
        Err(report) => violations.push(report),
        Ok(facts) => {
            if !facts.contains(required) {
                violations.push(format!("missing borrowed signature: {required}"));
            }
        }
    }

    violations
}

/// The process-substitution ownership edges, pinned where they are made.
///
/// Each finding here is one edge the audit walked: the name's scope has
/// a single opening, close-on-exec is cleared at a single site, the
/// substitution's own child disowns its parent's names, and the child is
/// forked without a job. Release is by `Drop` and nothing else, so no path
/// -- error, interrupt or early return -- can skip it.
// [spec:nsh:req:compat.bash.safe-core/test]
fn process_substitution_ownership_is_pinned() -> Vec<String> {
    let workspace = workspace_root();
    let module = "crates/nsh/src/evaluation/bash_process_substitution.rs";
    let mut violations = Vec::new();

    match text_at(&workspace, module) {
        Err(report) => violations.push(report),
        Ok(source) => {
            for required in [
                "pub(crate) struct SubstitutionStack(Arc<Mutex<Vec<Descriptor>>>)",
                "impl Drop for NameScope",
                "self.stack.open().truncate(self.mark)",
                "shell.process_substitutions.open().clear()",
                "crate::jobs::ForkMode::WithoutJob",
                "fork_shell(shell, None, None,",
                "drop(shell_end)",
                "drop(child_end)",
            ] {
                if !source.contains(required) {
                    violations.push(format!("{module} no longer states {required:?}"));
                }
            }
            for forbidden in [
                "ManuallyDrop",
                "std::mem::forget",
                "into_raw",
                "fn release(",
            ] {
                if source.contains(forbidden) {
                    violations.push(format!(
                        "{module} released a name outside Drop with {forbidden:?}"
                    ));
                }
            }
        }
    }

    let mut scopes = 0_usize;
    let mut publishes = 0_usize;
    for path in rust_sources_in(&workspace, "crates/nsh/src") {
        let text = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let relative = relative_to_workspace(&path, &workspace);
        let opened = text.matches("bash_process_substitution::scope(").count();
        let published = text
            .matches("bash_process_substitution::publish_before_exec(")
            .count();
        if opened != 0 && relative != "crates/nsh/src/evaluation.rs" {
            violations.push(format!("{relative} opens a second substitution scope"));
        }
        if published != 0 && relative != "crates/nsh/src/execution.rs" {
            violations.push(format!(
                "{relative} publishes substitution names outside the exec terminus"
            ));
        }
        scopes += opened;
        publishes += published;
    }
    if scopes != 1 {
        violations.push(format!(
            "the name scope has {scopes} openings rather than the one"
        ));
    }
    if publishes != 1 {
        violations.push(format!(
            "close-on-exec is cleared at {publishes} sites rather than the one"
        ));
    }

    match text_at(&workspace, "crates/nsh/src/execution.rs") {
        Err(report) => violations.push(report),
        Ok(execution) => {
            let publish = execution.find("bash_process_substitution::publish_before_exec(");
            let materialize = execution.find("shell.descriptors.materialize()");
            match (publish, materialize) {
                (None, _) => {
                    violations.push("the exec terminus no longer publishes".to_owned());
                }
                (_, None) => {
                    violations.push("the exec terminus no longer materializes".to_owned());
                }
                (Some(publish), Some(materialize)) if publish >= materialize => {
                    violations.push(
                        "names are published after the descriptor table is materialized".to_owned(),
                    );
                }
                _ => {}
            }
        }
    }

    violations
}

/// No GNU Bash or Readline code was copied into this workspace.
///
/// Two checks, because the rule has two failure modes. A copied file
/// brings its notice with it, so any Free Software Foundation or GPL
/// notice under `crates/` is a finding on its own. A copied *fragment*
/// brings the upstream's identifiers instead, so those are matched as
/// identifiers -- prose naming a Bash function in a comment is a citation,
/// which the scanner drops along with the rest of the comment.
///
/// The Readline reference tables are the one place upstream *data* is
/// reproduced, so they must carry a provenance record naming what produced
/// them and which version was observed.
// [spec:nsh:req:compat.bash.safe-core/test]
fn no_gnu_bash_code_was_copied() -> Vec<String> {
    const UPSTREAM_NOTICES: &[&str] = &[
        "Free Software Foundation",
        "GNU General Public License",
        "This file is part of GNU Bash",
        "This file is part of the GNU Readline",
        "SPDX-License-Identifier: GPL",
    ];
    const UPSTREAM_IDENTIFIERS: &[&str] = &[
        "rl_insert",
        "rl_bind_key",
        "rl_funmap_names",
        "rl_untranslate_keyseq",
        "emacs_standard_keymap",
        "emacs_meta_keymap",
        "vi_movement_keymap",
        "decode_prompt_string",
        "expand_word_internal",
        "execute_command_internal",
        "make_word_list",
        "dispose_words",
        "WORD_DESC",
        "SHELL_VAR",
        "sh_xmalloc",
        "internal_getopt",
        "builtin_usage",
    ];

    let workspace = workspace_root();
    let checker_crate = concat!("crates/", env!("CARGO_PKG_NAME"), "/");
    let mut violations = Vec::new();
    let mut sources = Vec::new();
    rust_sources_below(&workspace.join("crates"), &mut sources);
    sources.sort();

    for path in sources {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let relative = relative_to_workspace(&path, &workspace);
        /* This crate spells every notice out in order to look for it, so
         * it is the one place the search cannot include. Derived from the
         * manifest rather than written as a path: the literal it replaces
         * said `crates/nsh/tests/idiomatic_control_flow.rs` and went stale
         * the moment the checks moved, which is the whole reason a path
         * should never be spelled out where it can be asked for. */
        let checker = relative.starts_with(checker_crate);
        for notice in UPSTREAM_NOTICES {
            if !checker && source.contains(notice) {
                violations.push(format!("{relative} carries an upstream notice: {notice:?}"));
            }
        }
        let scan = scan_rust_source(&source);
        for identifier in UPSTREAM_IDENTIFIERS {
            if scan.identifiers.iter().any(|name| name == identifier) {
                violations.push(format!("{relative} names upstream symbol {identifier}"));
            }
        }
    }

    /* Readline is a separate GPL-3 project that Bash links, not Bash, and
     * this shell edits with `nshedit`. Reproducing Readline's function,
     * variable and key-map inventory therefore described a library that is
     * not here and advertised commands the editor cannot run. The tables
     * were removed rather than relicensed; this keeps them from returning
     * under any name. */
    if workspace
        .join("crates/nsh/src/builtins/bind/tables.rs")
        .exists()
    {
        violations.push("the Readline reference tables came back".to_owned());
    }
    for name in [
        "accept-line",
        "blink-matching-paren",
        "vi-subst",
        "menu-complete",
    ] {
        for path in ["crates/nsh/src", "crates/nsh-cli/src"] {
            let mut found = Vec::new();
            rust_sources_below(&workspace.join(path), &mut found);
            for source_path in found {
                let text = std::fs::read_to_string(&source_path).expect("Rust source is UTF-8");
                if text.contains(name) {
                    violations.push(format!(
                        "{} names the Readline command {name}",
                        relative_to_workspace(&source_path, &workspace)
                    ));
                }
            }
        }
    }

    violations
}

/// The expression engine bounds its own work, in steps and in stack.
///
/// A step budget alone does not bound recursion: a quantified group takes
/// one continuation frame per subject character, so a long subject reaches
/// the budget only after the stack is already gone. Both bounds are read
/// in the one place every recursion passes through, so nesting cannot get
/// around either.
// [spec:nsh:req:compat.bash.safe-core/test]
fn the_expression_engine_bounds_its_work() -> Vec<String> {
    let workspace = workspace_root();
    let source = match text_at(&workspace, "crates/nsh/src/regex.rs") {
        Ok(source) => source,
        Err(report) => return vec![report],
    };
    let scan = scan_rust_source(&source);

    let mut reported = Vec::new();
    if scan.identifiers.iter().any(|name| name == "unsafe") {
        reported.push("the expression engine is no longer safe Rust".to_owned());
    }
    for required in [
        "const STEP_BUDGET: u64",
        "const MAX_DEPTH: u32",
        "steps: &'a mut u64",
        "depth: u32",
        "if *self.steps > STEP_BUDGET || self.depth >= MAX_DEPTH {",
    ] {
        if !source.contains(required) {
            reported.push(format!("the engine lost its bound: {required:?}"));
        }
    }
    for (bound, what) in [
        ("STEP_BUDGET", "the step budget"),
        ("MAX_DEPTH", "the depth bound"),
    ] {
        let occurrences = source.matches(bound).count();
        if occurrences != 2 {
            reported.push(format!(
                "{what} is named {occurrences} times rather than twice, so it is \
                 read somewhere other than its single guard"
            ));
        }
    }

    match source
        .split_once("pub(crate) fn search(")
        .and_then(|(_, after)| after.split_once("    fn match_at("))
    {
        None => {
            reported.push("the engine has no search followed by the attempt it makes".to_owned())
        }
        Some((search, _)) => {
            if !search.contains("let mut steps = 0_u64;") || !search.contains("&mut steps") {
                reported.push(
                    "the search no longer buys a fresh budget at every start offset".to_owned(),
                );
            }
        }
    }
    reported
}

/// One source-shape check: it reads the repository and answers with what it
/// found, which is nothing when the shape is right.
type Check = fn() -> Vec<String>;

/// Run every check, and report every finding rather than the first.
///
/// Until 2026-09-02 a check asserted and `main` ran it under
/// `catch_unwind`, which was how these were written when they were
/// `#[test]` functions. That cost three things. `Cargo.toml` says nothing
/// in this workspace relies on unwinding, and it stopped being true: under
/// `panic = "abort"` the first failing check would have taken the other
/// twenty-five with it. An empty panic hook had to be installed so the
/// runtime would not print each failure beside the report that already
/// carried it, and that hook swallowed the location of a genuine bug in
/// the checker too -- a checker that crashed and a check that failed
/// looked the same, and neither said where. And an assertion
/// stops at the first thing it finds, so a run reported one fossil out of
/// however many there were, and finding the next one cost another run.
///
/// A check returns its findings instead. Every check runs, every finding
/// is reported, one per line, named by the check that made it.
fn main() {
    let checks: &[(&str, Check)] = &[
        ("port_fossils_are_absent", port_fossils_are_absent),
        (
            "core_enforces_strict_rust_lints",
            core_enforces_strict_rust_lints,
        ),
        (
            "zero_cism_gate_covers_boundary",
            zero_cism_gate_covers_boundary,
        ),
        (
            "subsystem_helpers_use_narrow_state",
            subsystem_helpers_use_narrow_state,
        ),
        ("output_failures_are_returned", output_failures_are_returned),
        (
            "fallible_results_are_explicit",
            fallible_results_are_explicit,
        ),
        (
            "dynamic_values_are_not_clamped",
            dynamic_values_are_not_clamped,
        ),
        (
            "builtin_registry_is_fully_typed",
            builtin_registry_is_fully_typed,
        ),
        (
            "shell_entrypoint_uses_public_runtime",
            shell_entrypoint_uses_public_runtime,
        ),
        (
            "modules_follow_rust_subsystems",
            modules_follow_rust_subsystems,
        ),
        ("mystring_module_is_absent", mystring_module_is_absent),
        (
            "core_strings_are_length_delimited",
            core_strings_are_length_delimited,
        ),
        ("core_avoids_abi_scalars", core_avoids_abi_scalars),
        ("control_flow_is_structured", control_flow_is_structured),
        ("operation_modes_are_typed", operation_modes_are_typed),
        (
            "evaluator_control_is_carried_by_flow",
            evaluator_control_is_carried_by_flow,
        ),
        (
            "jobs_read_startup_are_structured",
            jobs_read_startup_are_structured,
        ),
        ("typed_job_control_model", typed_job_control_model),
        ("typed_shell_options", typed_shell_options),
        (
            "the_compatibility_delta_stays_safe",
            the_compatibility_delta_stays_safe,
        ),
        (
            "platform_unsafe_is_named_and_justified",
            platform_unsafe_is_named_and_justified,
        ),
        (
            "descriptors_cross_the_boundary_owned",
            descriptors_cross_the_boundary_owned,
        ),
        (
            "process_substitution_ownership_is_pinned",
            process_substitution_ownership_is_pinned,
        ),
        ("no_gnu_bash_code_was_copied", no_gnu_bash_code_was_copied),
        (
            "the_expression_engine_bounds_its_work",
            the_expression_engine_bounds_its_work,
        ),
        (
            "files_near_the_cap_are_registered",
            density::files_near_the_cap_are_registered,
        ),
        (
            "cited_decisions_are_live",
            citations::cited_decisions_are_live,
        ),
    ];
    let mut failed = 0_usize;
    let mut findings = Vec::new();
    for (name, check) in checks {
        let reports = check();
        if reports.is_empty() {
            continue;
        }
        failed += 1;
        findings.extend(
            reports
                .into_iter()
                .map(|report| format!("{name}: {report}")),
        );
    }
    if findings.is_empty() {
        println!("nsh-lint: {} source-shape checks, all clean", checks.len());
        return;
    }
    eprintln!(
        "nsh-lint: {} of {} checks failed, {} findings",
        failed,
        checks.len(),
        findings.len()
    );
    for finding in &findings {
        eprintln!("  {finding}");
    }
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module read has to reach the files beside the one it names.
    ///
    /// This is the property the old `include_str!` did not have, and the
    /// one worth pinning: `record_child_status` lives in `jobs/wait.rs`
    /// and `JobState` in `jobs/model.rs`, so a read that only opened
    /// `jobs.rs` would find neither -- and a forbidden shape put in either
    /// file would go unreported.
    #[test]
    fn a_module_read_reaches_the_files_beside_it() {
        let jobs = module("nsh/src/jobs");
        assert!(
            jobs.contains("fn record_child_status"),
            "missing jobs/wait.rs"
        );
        assert!(jobs.contains("enum JobState"), "missing jobs/model.rs");
        assert!(jobs.contains("fn create_job"), "missing jobs.rs itself");
    }

    /// A module that is not there is a failure, not an empty string.
    ///
    /// `read_to_string` answers a missing file with an `Err` that
    /// `unwrap_or_default` would turn into "nothing forbidden here".
    #[test]
    #[should_panic(expected = "no source at")]
    fn a_module_that_is_not_there_is_refused() {
        let _ = module("nsh/src/no-such-subsystem");
    }
}
