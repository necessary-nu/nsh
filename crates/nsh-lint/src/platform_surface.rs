//! The two hosts' published names, read off the source and compared.
//!
//! No `cfg(target_os ...)` is permitted in the shell, so the shell names
//! `nsh_platform::LocaleCharacter` once and expects it to exist
//! everywhere. A host whose table of contents is one name short therefore
//! fails in the *shell*, on a target nobody here builds, rather than in
//! the file that is short -- which is how `LocaleCharacter` sat
//! unpublished on Windows from `abe28ce` to `17a7b6a` with every local
//! check green.
//!
//! Nothing the compiler does can catch that, because the compiler only
//! ever sees one host. Compiling for the other one would, and the two
//! Windows targets are installed here; but a check that depends on a
//! target being installed is a check that can silently not run, which is
//! the failure this repository kept finding. The two lists are text, and
//! text can be compared on any host.
//!
//! WHAT IS COMPARED. A top-level `pub use` in a host's table of contents,
//! against the `cfg` conditions stacked on it -- a name published under
//! `feature = "edit"` on one host and unconditionally on the other has
//! drifted as surely as one that is missing. The crate root's own
//! `pub use` items join the comparison when they carry a host predicate.
//! Nothing in the crate root carries one today -- each host's table of
//! contents lists its whole surface -- but the merge stays, because a
//! name published from the root would otherwise be on a surface without
//! appearing in the file that lists it.
//!
//! WHAT IS NOT, said here rather than left to be discovered. This reads
//! module-level names, so a `cfg`-split *inherent method* -- of which
//! `Locale::character_encoding` in `lib.rs` is one -- is invisible to it,
//! as are trait implementations, enum variants and struct fields. A
//! `pub` item declared in a host root by any route other than `pub use`
//! is not compared either, and is reported as such rather than skipped:
//! a surface this check cannot read makes it go red, not quiet.

use std::collections::{BTreeMap, BTreeSet};

use crate::workspace_root;

/// The platform crate's source directory, relative to the workspace root.
const PLATFORM: &str = "crates/nsh-platform/src";

/// The crate root, read because a host-guarded `pub use` written here
/// would belong to one surface and not the other.
const CRATE_ROOT: &str = "lib.rs";

/// Every supported host: the `cfg` predicate that selects it in the crate
/// root, and the file that is its table of contents.
const HOSTS: &[(&str, &str)] = &[("unix", "unix.rs"), ("windows", "windows.rs")];

/// What one host publishes: each name, against the conditions it is
/// published under, sorted and joined. An empty condition reads "on this
/// host, in every build".
type Surface = BTreeMap<String, String>;

/// One `pub use` of one name, before either host has claimed it: the name
/// and the conditions that publication carries.
///
/// A list rather than a map, because one file may publish a name twice
/// -- once under `unix` and once under `windows` -- and folding those
/// into one entry loses whichever host is read first, which is a name
/// reported missing from a host that has it. `a_table_of_contents_reads`
/// is where that is measured; no file in the platform crate is written
/// that way today, so nothing else would notice the fold.
type Publication = (String, String);

/// Every host's table of contents publishes the same names under the same
/// conditions, reported as findings.
// [spec:nsh:req:idiom.platform-surface-parity/test]
pub(crate) fn hosts_publish_the_same_surface() -> Vec<String> {
    let source = workspace_root().join(PLATFORM);
    let mut reported = Vec::new();
    let root = match std::fs::read_to_string(source.join(CRATE_ROOT)) {
        Ok(text) => published(&text),
        Err(error) => return vec![format!("{PLATFORM}/{CRATE_ROOT} is not readable: {error}")],
    };
    let mut surfaces = Vec::new();
    for (host, file) in HOSTS {
        let text = match std::fs::read_to_string(source.join(file)) {
            Ok(text) => text,
            Err(error) => {
                reported.push(format!("{PLATFORM}/{file} is not readable: {error}"));
                continue;
            }
        };
        let (host_published, opaque) = published(&text);
        let mut surface: Surface = host_published.into_iter().collect();
        for (conditions, item) in opaque {
            reported.push(format!(
                "{file} publishes `{item}` by a route this check cannot compare; \
                 publish it with a `pub use` or teach the check to read it \
                 (conditions: {})",
                describe(&conditions)
            ));
        }
        /* The crate root's unconditional items are on both hosts by
         * construction and cancel out; only the ones it guards with a
         * host predicate belong to one surface and not the other. */
        for (name, conditions) in &root.0 {
            if let Some(rest) = without(conditions, host) {
                surface.insert(name.clone(), rest);
            }
        }
        for (conditions, item) in &root.1 {
            if without(conditions, host).is_some() {
                reported.push(format!(
                    "{CRATE_ROOT} publishes `{item}` for {host} alone by a route \
                     this check cannot compare"
                ));
            }
        }
        surfaces.push((*host, surface));
    }
    if surfaces.len() == HOSTS.len() {
        let (first, left) = &surfaces[0];
        for (second, right) in &surfaces[1..] {
            reported.extend(drift((first, left), (second, right)));
        }
    }
    reported
}

fn drift(left: (&str, &Surface), right: (&str, &Surface)) -> Vec<String> {
    let ((first, left), (second, right)) = (left, right);
    let mut reported = Vec::new();
    let names: BTreeSet<_> = left.keys().chain(right.keys()).collect();
    for name in names {
        match (left.get(name), right.get(name)) {
            (Some(here), Some(there)) if here == there => {}
            (Some(here), Some(there)) => reported.push(format!(
                "`{name}` is published {} on {first} and {} on {second}",
                describe(here),
                describe(there)
            )),
            (Some(_), None) => reported.push(format!(
                "`{name}` is published on {first} and not on {second}"
            )),
            (None, Some(_)) => reported.push(format!(
                "`{name}` is published on {second} and not on {first}"
            )),
            (None, None) => unreachable!("a name came from one of the two maps"),
        }
    }
    reported
}

/// Every top-level `pub use` in one file, and every other top-level `pub`
/// item beside it -- the second so that a publication this cannot read is
/// reported rather than passed over.
///
/// An item counts only at column zero. That is what stands in for brace
/// tracking: every top-level item in these files is unindented and
/// everything inside a module, a function or a test is not, so a
/// `pub use` in a test module cannot be mistaken for a published name,
/// and a brace inside a string literal cannot throw the reading off.
fn published(text: &str) -> (Vec<Publication>, Vec<(String, String)>) {
    let mut surface = Vec::new();
    let mut opaque = Vec::new();
    let mut conditions: Vec<String> = Vec::new();
    let mut item = String::new();
    for line in code_only(text).lines() {
        let line = line.trim_end();
        if item.is_empty() {
            if line.is_empty() || line.starts_with(char::is_whitespace) {
                continue;
            }
            if let Some(condition) = line
                .strip_prefix("#[cfg(")
                .and_then(|rest| rest.strip_suffix(")]"))
            {
                conditions.extend(predicates(condition));
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            if !line.starts_with("pub ") {
                conditions.clear();
                continue;
            }
            if !line.starts_with("pub use ") {
                conditions.sort();
                opaque.push((
                    conditions.join(" + "),
                    line.trim_end_matches(['{', ' ']).to_owned(),
                ));
                conditions.clear();
                continue;
            }
        }
        item.push_str(line);
        if !item.contains(';') {
            item.push(' ');
            continue;
        }
        conditions.sort();
        let condition = conditions.join(" + ");
        for name in names_in(&item) {
            surface.push((name, condition.clone()));
        }
        conditions.clear();
        item.clear();
    }
    (surface, opaque)
}

/// One `cfg` attribute's conditions, with an `all(...)` opened up into
/// the predicates it conjoins.
///
/// `#[cfg(all(unix, feature = "edit"))]` and the same two written as
/// separate attributes mean one thing to the compiler, so they have to
/// mean one thing here: a host predicate hidden inside an `all` would
/// otherwise leave its names on neither surface, and be reported missing
/// from the host that has them.
fn predicates(condition: &str) -> Vec<String> {
    let Some(inner) = condition
        .trim()
        .strip_prefix("all(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return vec![condition.trim().to_owned()];
    };
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (at, character) in inner.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.extend(predicates(&inner[start..at]));
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.extend(predicates(&inner[start..]));
    parts
}

/// The names one `pub use` item publishes.
///
/// A glob is not a name: both hosts' contents reach the crate root
/// through one, and what the glob carries is exactly what the lists below
/// it say.
fn names_in(item: &str) -> Vec<String> {
    let body = item
        .trim_start_matches("pub use ")
        .trim()
        .trim_end_matches(';');
    let listed = match body.split_once('{') {
        Some((_, tail)) => tail.trim_end().trim_end_matches('}'),
        None => body.rsplit("::").next().unwrap_or(body),
    };
    listed
        .split(',')
        .map(|name| name.rsplit(" as ").next().unwrap_or(name).trim())
        .filter(|name| !name.is_empty() && *name != "*")
        .map(str::to_owned)
        .collect()
}

/// The conditions left over once `host` is struck out, or `None` when
/// `host` was not among them.
fn without(conditions: &str, host: &str) -> Option<String> {
    let mut rest: Vec<&str> = Vec::new();
    let mut found = false;
    for condition in conditions.split(" + ").filter(|part| !part.is_empty()) {
        if condition == host {
            found = true;
        } else {
            rest.push(condition);
        }
    }
    found.then(|| rest.join(" + "))
}

fn describe(conditions: &str) -> String {
    if conditions.is_empty() {
        "in every build".to_owned()
    } else {
        format!("under `{conditions}`")
    }
}

/// The file with its comments blanked out, line structure kept.
///
/// A doc comment quoting a `pub use` is a real thing in these files, and
/// reading one as a publication would put a name on a surface that does
/// not have it.
fn code_only(text: &str) -> String {
    let mut kept = Vec::with_capacity(text.len());
    let mut depth = 0_usize;
    for line in text.lines() {
        let bytes = line.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            if depth == 0 && bytes[at..].starts_with(b"//") {
                break;
            }
            if bytes[at..].starts_with(b"/*") {
                depth += 1;
                at += 2;
                continue;
            }
            if depth > 0 && bytes[at..].starts_with(b"*/") {
                depth -= 1;
                at += 2;
                continue;
            }
            if depth == 0 {
                kept.push(bytes[at]);
            }
            at += 1;
        }
        kept.push(b'\n');
    }
    String::from_utf8(kept).expect("dropping whole comments leaves the rest as it was")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `pub use` per shape the platform roots actually write, the
    /// three shapes that must not become names, and the one name two
    /// hosts publish from the same file.
    #[test]
    fn a_table_of_contents_reads() {
        let (published_names, opaque) = published(
            "mod locale;\n\
             pub use locale::{Locale, LocaleCharacter};\n\
             #[cfg(feature = \"edit\")]\n\
             pub use editor_terminal::{TerminalApply, editor_terminal_size};\n\
             pub use spawn::execute_program;\n\
             pub use paths::{\n    AccessMode,\n    absolute_path,\n};\n\
             pub use windows::*;\n\
             pub use text::NativeStrExt as Native;\n\
             #[cfg(unix)]\n\
             pub use unix::facts::UserId;\n\
             #[cfg(windows)]\n\
             pub use windows::facts::UserId;\n\
             #[cfg(test)]\n\
             mod tests {\n    pub use hidden::NotPublished;\n}\n",
        );
        let condition = |wanted: &str| -> Vec<&str> {
            published_names
                .iter()
                .filter(|(name, _)| name == wanted)
                .map(|(_, conditions)| conditions.as_str())
                .collect()
        };
        assert_eq!(condition("Locale"), [""]);
        assert_eq!(condition("LocaleCharacter"), [""]);
        assert_eq!(condition("editor_terminal_size"), ["feature = \"edit\""]);
        assert_eq!(condition("execute_program"), [""]);
        assert_eq!(condition("absolute_path"), [""]);
        assert_eq!(condition("Native"), [""]);
        assert_eq!(
            condition("UserId"),
            ["unix", "windows"],
            "a name two hosts publish is two publications, not one"
        );
        assert!(condition("*").is_empty(), "a glob is not a name");
        assert!(
            condition("NotPublished").is_empty(),
            "an indented `pub use` is not a top-level publication"
        );
        assert!(opaque.is_empty(), "{opaque:?}");
    }

    #[test]
    fn a_publication_this_cannot_read_is_reported() {
        let (surface, opaque) = published("#[cfg(unix)]\npub fn host_name() -> String {\n}\n");
        assert!(surface.is_empty());
        assert_eq!(
            opaque,
            vec![("unix".to_owned(), "pub fn host_name() -> String".to_owned())]
        );
    }

    /// The three ways two surfaces can disagree, and the one way they can
    /// agree.
    #[test]
    fn drift_is_a_missing_name_or_a_condition() {
        let surface = |pairs: &[(&str, &str)]| -> Surface {
            pairs
                .iter()
                .map(|(name, condition)| ((*name).to_owned(), (*condition).to_owned()))
                .collect()
        };
        let left = surface(&[("Locale", ""), ("LocaleCharacter", ""), ("Apply", "f")]);
        let right = surface(&[("Locale", ""), ("Apply", "")]);
        let findings = drift(("unix", &left), ("windows", &right));
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(findings[0].contains("`Apply` is published under `f` on unix and in every build"));
        assert!(findings[1].contains("`LocaleCharacter` is published on unix and not on windows"));
        assert!(drift(("unix", &left), ("unix", &left)).is_empty());
    }

    /// The platform crate this check is about is read, and read whole --
    /// the property that would break silently if the roots were rewritten
    /// into a shape the reader above does not recognise.
    #[test]
    fn the_platform_roots_read() {
        let source = workspace_root().join(PLATFORM);
        for (_, file) in HOSTS {
            let text = std::fs::read_to_string(source.join(file)).expect("a host root is readable");
            let (names, opaque) = published(&text);
            assert!(
                names.len() > 100,
                "{file} read as {} published names",
                names.len()
            );
            assert!(opaque.is_empty(), "{file}: {opaque:?}");
        }
    }

    /// Each host's surface is listed in that host's own table of
    /// contents, and the crate root adds nothing to either.
    ///
    /// The root publishes each host's module whole and names nothing
    /// under a host predicate itself, so a reader who wants to know what
    /// a host publishes has one file to open. A name published from the
    /// root instead is on a surface without appearing in the file that is
    /// supposed to list it, which is the drift this check exists to
    /// catch, one level up: the two lists would still agree with each
    /// other and neither would be the whole truth.
    ///
    /// The merging in `hosts_publish_the_same_surface` stays, because
    /// nothing stops a root publication being written again -- and this
    /// is what would report it. `a_table_of_contents_reads` is where the
    /// merge's hazard is measured, on a literal that holds `UserId` under
    /// both hosts whatever the platform crate happens to look like today.
    #[test]
    fn the_crate_root_adds_nothing_to_a_surface() {
        let text = std::fs::read_to_string(workspace_root().join(PLATFORM).join(CRATE_ROOT))
            .expect("the crate root is readable");
        let (names, opaque) = published(&text);
        for (host, file) in HOSTS {
            let guarded: Vec<_> = names
                .iter()
                .filter_map(|(name, conditions)| without(conditions, host).map(|_| name.as_str()))
                .collect();
            assert!(
                guarded.is_empty(),
                "{CRATE_ROOT} publishes {guarded:?} for {host} alone; \
                 they belong in {file}"
            );
            /* Only the host-guarded ones. The root's unguarded `pub`
             * items -- `mod facts`, `enum CharacterEncoding` -- are on
             * both hosts by construction and are not a surface split. */
            let cannot_read: Vec<_> = opaque
                .iter()
                .filter(|(conditions, _)| without(conditions, host).is_some())
                .map(|(_, item)| item.as_str())
                .collect();
            assert!(
                cannot_read.is_empty(),
                "{CRATE_ROOT} publishes {cannot_read:?} for {host} alone \
                 by a route the check cannot compare"
            );
        }
    }
}
