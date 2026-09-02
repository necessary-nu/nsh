//! The density bill, presented five functions before it is due.
//!
//! `[spec:np:req:dod.function-density]` caps a staged file at fifty
//! functions and charges *growth* rather than level, so a file already at
//! fifty costs nothing until somebody adds to it -- and then it costs a
//! split, presented as a refused commit during whatever change happened to
//! arrive. This reports at forty-five instead, against a checked-in
//! register whose entries say what the answer is.
//!
//! The register is symmetric, exactly as `BASH_DISPOSITIONS.toml` is: a
//! file at or above the mark with no entry is reported, and a registered
//! file that has fallen back below the mark is reported until its entry is
//! deleted. One half without the other is a place to hide.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::{
    character_literal_end, relative_to_workspace, rust_sources_in, string_literal_end,
    workspace_root,
};

/// The function count at or above which a file must be registered.
const MARK: u32 = 45;

/// The cap `[spec:np:req:dod.function-density]` enforces, from
/// `.config/nplan/config.styx`.
const CAP: u32 = 50;

/// Where the register lives, relative to the workspace root.
const REGISTER: &str = "crates/nsh-lint/FUNCTION_DENSITY.toml";

/// The tree the register covers.
///
/// The gate judges any staged file, and `fuzz/` is Rust too; it is left
/// out because its largest file holds six functions, and adding it is one
/// more entry in this constant on the day that stops being true.
const TREE: &str = "crates";

/// The register, as it is written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    schema: u32,
    /// Repeated from [`MARK`] so the file says what it means; checked
    /// against it below so the two cannot drift.
    mark: u32,
    /// Likewise for [`CAP`].
    cap: u32,
    #[serde(default)]
    file: Vec<Entry>,
}

/// One registered file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    /// What to do about it: the seam to split along, the helpers to fold,
    /// or why the file is right as it is. Not a restatement of its size.
    answer: String,
}

/// Every file at or above the mark is registered, and every register entry
/// is still about a file at or above the mark, reported as findings.
///
/// Deliberately unannotated. Every other check in this crate carries a
/// `[spec:nsh:req:...]` reference because it enforces a rule this
/// repository's own spec states; the rule this one gives warning of is
/// nplan's `dod.function-density`, which belongs to the tool rather than
/// to the shell, and claiming coverage of it from here would be a lie
/// about who implements what.
pub(crate) fn files_near_the_cap_are_registered() -> Vec<String> {
    let workspace = workspace_root();
    let text = match std::fs::read_to_string(workspace.join(REGISTER)) {
        Ok(text) => text,
        Err(error) => return vec![format!("{REGISTER} is not readable: {error}")],
    };
    let register: Register = match toml::from_str(&text) {
        Ok(register) => register,
        Err(error) => return vec![format!("{REGISTER} does not parse: {error}")],
    };

    let mut reported = Vec::new();
    if register.schema != 1 {
        reported.push(format!(
            "{REGISTER} states schema {}, which this check does not know",
            register.schema
        ));
    }
    if register.mark != MARK {
        reported.push(format!(
            "{REGISTER} states a mark of {}, not the {MARK} this check enforces",
            register.mark
        ));
    }
    if register.cap != CAP {
        reported.push(format!(
            "{REGISTER} states a cap of {}, not the {CAP} the density gate enforces",
            register.cap
        ));
    }
    let mut registered = BTreeSet::new();
    for entry in &register.file {
        if !registered.insert(entry.path.as_str()) {
            reported.push(format!("{} is registered twice", entry.path));
        }
        if entry.answer.trim().is_empty() {
            reported.push(format!(
                "{} is registered with no answer; name the seam to split \
                 along, the helpers to fold, or why the file is right as it is",
                entry.path
            ));
        }
    }

    /* The checker's own source is counted like any other. Five of these
     * checks were retargeted at this crate when they moved here and found
     * the needles they carry as constants; this one wants exactly that,
     * because `main.rs` is a file under `crates/` and the gate will charge
     * it the same fifty as the shell's. */
    let mut counted = BTreeMap::new();
    for path in rust_sources_in(&workspace, TREE) {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        counted.insert(
            relative_to_workspace(&path, &workspace),
            rust_function_items(&source),
        );
    }

    for (path, count) in &counted {
        if *count >= MARK && !registered.contains(path.as_str()) {
            reported.push(format!(
                "{path} holds {count} functions, at or above the mark of {MARK}, \
                 with no entry in {REGISTER}; the cap is {CAP} and it is charged \
                 on the next change that adds one, so decide now and write the \
                 decision down"
            ));
        }
    }
    for path in &registered {
        match counted.get(*path) {
            None => reported.push(format!(
                "{path} is registered in {REGISTER} but there is no such file \
                 under {TREE}/; delete the entry or fix the path"
            )),
            Some(count) if *count < MARK => reported.push(format!(
                "{path} holds {count} functions, below the mark of {MARK}, and is \
                 still registered in {REGISTER}; delete the entry -- a register \
                 that keeps stale excuses is where a real one gets waved through"
            )),
            Some(_) => {}
        }
    }

    reported
}

/// How many `function_item`s a Rust file holds.
///
/// This has to agree with the gate or the warning is worse than nothing.
/// The gate counts tree-sitter `function_item` nodes anywhere in the file
/// -- so a method in an `impl`, a trait method that has a body, a function
/// nested inside another, and a `#[test]` in the file's own
/// `mod tests { ... }` all count, because `exempt-test-scope` is false.
/// A trait's bodiless `fn f(&self);` and an `extern "C"` block's
/// declaration are `function_signature_item`s and a `fn(u32) -> u32` in
/// type position is a `function_type`; the gate counts none of the three,
/// and neither does this. Nor does either count a `fn` inside a macro's
/// token tree, which is not parsed as an item at all.
fn rust_function_items(source: &str) -> u32 {
    let bytes = blanked_rust_source(source);
    let mut at = 0;
    let mut functions = 0;
    while at < bytes.len() {
        if !is_identifier_byte(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        at = identifier_end(&bytes, at);
        if let Some(end) = macro_token_tree_end(&bytes, at) {
            at = end;
        } else if &bytes[start..at] == b"fn" && function_item_follows(&bytes, at) {
            functions += 1;
        }
    }
    functions
}

/// The source with every comment, string and character literal replaced by
/// spaces of the same length.
///
/// A `fn` in a doc comment's example, in a diagnostic's text, or in a
/// literal this crate carries in order to search for it must not reach the
/// count. Positions are preserved so the scan that follows can be a plain
/// walk.
fn blanked_rust_source(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut blanked = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"//") {
            let end = at
                + bytes[at..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .unwrap_or(bytes.len() - at);
            blanked[at..end].fill(b' ');
            at = end;
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            let start = at;
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
            blanked[start..at].fill(b' ');
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
            blanked[at..end].fill(b' ');
            at = end;
            continue;
        }
        if let Some((end, _)) = string_literal_end(bytes, at) {
            blanked[at..end].fill(b' ');
            at = end;
            continue;
        }
        at += 1;
    }
    blanked
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn identifier_end(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(|byte| is_identifier_byte(*byte)) {
        at += 1;
    }
    at
}

fn skip_blanks(bytes: &[u8], mut at: usize) -> usize {
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// Whether the `fn` that ends at `after` introduces a function with a body.
///
/// A named declaration follows `fn` with an identifier -- a
/// `fn(u32) -> u32` type has a `(` there instead -- and then reaches
/// either a `{` at bracket depth zero, which is the body, or a `;`, which
/// makes it a signature the gate does not count.
fn function_item_follows(bytes: &[u8], after: usize) -> bool {
    let mut at = skip_blanks(bytes, after);
    if !bytes
        .get(at)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        return false;
    }
    /* `(`, `[` and `{` only: angle brackets are not delimiters and
     * counting them would mistake `->` and `>=` for one. A `;` inside a
     * parameter's `[u8; 4]` is therefore already at a depth above zero,
     * which is the only place a `;` can hide before the body. */
    let mut depth = 0i32;
    while at < bytes.len() {
        match bytes[at] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => return true,
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b';' if depth == 0 => return false,
            _ => {}
        }
        at += 1;
    }
    false
}

/// The end of the token tree of a macro whose name ends at `after`, if that
/// is what follows.
///
/// `macro_rules! m { fn made_by_macro() {} }` and `write!(f, "fn")` hold
/// their contents as token trees rather than as items, so tree-sitter
/// finds no `function_item` in either and the gate counts none. The `!=`
/// operator is the one thing that looks like this and is not it.
fn macro_token_tree_end(bytes: &[u8], after: usize) -> Option<usize> {
    if bytes.get(after) != Some(&b'!') || bytes.get(after + 1) == Some(&b'=') {
        return None;
    }
    let mut at = skip_blanks(bytes, after + 1);
    /* `macro_rules! name { .. }` puts the name between the `!` and the
     * tree; a plain invocation does not. */
    if bytes
        .get(at)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        at = skip_blanks(bytes, identifier_end(bytes, at));
    }
    let (open, close) = match *bytes.get(at)? {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0usize;
    while at < bytes.len() {
        if bytes[at] == open {
            depth += 1;
        } else if bytes[at] == close {
            depth -= 1;
            if depth == 0 {
                return Some(at + 1);
            }
        }
        at += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes that separate this count from a `grep -c 'fn '`.
    ///
    /// Every one of them was checked against the gate's own counter --
    /// tokei's `normalized.functions` -- on a file holding exactly this
    /// source, which answered 8.
    #[test]
    fn items_are_counted_not_the_word() {
        let source = r#"
//! fn in_a_doc_comment() {}
/* fn in_a_block_comment() {} */
const NAME: &str = "fn in_a_string() {}";
type Handler = fn(u32) -> u32;
type Handler2 = unsafe extern "C" fn(u32) -> u32;

pub trait T {
    fn signature_only(&self) -> u32;
    fn with_default(&self) -> u32 { 1 }
    fn where_signature(&self) -> u32 where Self: Sized;
}

unsafe extern "C" {
    fn foreign_one(a: u32) -> u32;
    fn foreign_two(a: u32) -> u32;
}

struct S;
impl S {
    pub fn method(&self, buf: [u8; 4]) -> u32 { buf[0] as u32 }
    const fn const_method() -> u32 { 0 }
}

pub async unsafe fn asynchronous() {}

fn outer() -> u32 {
    fn nested() -> u32 { 3 }
    let closure = |x: u32| x + 1;
    nested() + closure(1)
}

macro_rules! not_a_fn {
    () => {
        fn made_by_macro() {}
    };
}
not_a_fn!();

#[cfg(test)]
mod tests {
    #[test]
    fn a_test() {}
    #[test]
    fn another_test() {}
}
"#;
        assert_eq!(rust_function_items(source), 8);
    }

    /// A `!=` is not a macro, and skipping a token tree from one would
    /// swallow whatever function came next.
    #[test]
    fn a_comparison_is_not_a_macro() {
        assert_eq!(
            rust_function_items("fn f() { if a!= b { c(); } }\nfn g() {}"),
            2
        );
    }
}
