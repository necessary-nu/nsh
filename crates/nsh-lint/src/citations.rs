//! A reason written beside the code, checked against the plan that holds it.
//!
//! Four stale reasons were found in the week of 2026-08-25, each a refusal
//! or a limitation recorded next to the code it justified, with a
//! justification that had stopped being true. The one this file exists for
//! was `evaluation/timed.rs`, which refused `TIMEFORMAT` citing the
//! obsolesced decision `no-format-interpreters`. The refusal was still
//! right, but under a successor that says something narrower -- and it was
//! found by hand, six days late, while re-measuring for an unrelated
//! reason.
//!
//! That one is mechanical, and it is the only one of the four that is. A
//! comment citing `dec:nsh:...` for a decision the corpus no longer holds
//! in force is a defect the plan can answer without running anything.
//!
//! WHAT THIS DOES NOT REACH, said here because the check's silence would
//! otherwise be read as covering the class it was filed under. The other
//! three instances were prose *about code* rather than citations: a
//! survey disposition blocked on `[DescriptorSlot; 10]` after the table
//! became a `BTreeMap`, a blocker naming two segfaults three commits had
//! already fixed, and a case registered as unimplementable "because this
//! tree does not keep the braces" after the tree started keeping them.
//! Nothing here reads those and no honest check does. This one reads
//! names, not sentences: it cannot tell whether a live decision still says
//! what the comment claims it says, only whether the decision is still
//! standing.
//!
//! WHY THIS READS THE FILES RATHER THAN ASKING `nplan`. The tool would
//! answer the question directly, and `nplan check` is what runs this
//! crate -- so shelling out would make the `idiom` check depend on the
//! program invoking it, and fail to run wherever that program is not on
//! `PATH`. A check that can fail to run is the failure this repository has
//! been finding all week, five times over. The corpus is checked in beside
//! the source, every other check in this crate reads files, and this one
//! reads the same files the tool would. The cost of that choice is a
//! second parser for the frontmatter, so a corpus file this one cannot
//! read is reported rather than skipped: a schema change makes the check
//! go red, not quiet.
//!
//! No citation is spelled out in full anywhere in this file, and that is
//! deliberate rather than shy -- the sweep below reads every file under
//! `crates/`, its own included, so a citation written here would be a
//! citation like any other and would be reported like one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{relative_to_workspace, workspace_root};

/// Where the decision corpus lives, relative to the workspace root.
const CORPUS: &str = "plan/decisions";

/// The tree whose citations are checked.
const TREE: &str = "crates";

/// The states in which a decision is still standing, and so may be cited
/// as a reason.
///
/// `decided` is "committed but not yet formally signed off" and `approved`
/// is "signed off and in force"; nplan's own `requires` edge presupposes
/// exactly those two, which is the closest thing to a definition of "in
/// force" the schema states. `challenged` is here as well because it means
/// "approved but under active reconsideration" -- the decision has not
/// stopped binding, and reporting every citation of it the moment somebody
/// marks a reconsideration would punish the record-keeping this check
/// depends on. `idea`, `tentative`, `rejected` and `obsolesced` are
/// not in force: two were never binding, and two are terminal.
const LIVE: &[&str] = &["@decided", "@approved", "@challenged"];

/// A decision as the corpus records it.
struct Decision {
    /// Its `state` field, `@`-sigil and all.
    state: String,
    /// The decisions whose `edges.supersedes` names this one. The corpus
    /// records the replacement from the successor's side, so this is
    /// gathered by reading every file rather than from the dead one.
    superseded_by: Vec<String>,
}

/// One citation, and enough about where it sits to tell whether the
/// correction is beside it.
struct Citation {
    /// One-based, for a message somebody can jump to.
    line: usize,
    /// The comment block the citation sits in. See [`citations`].
    block: usize,
    /// Canonical form -- a bracketed `dec`, corpus prefix and name --
    /// whatever the spelling at the site was.
    id: String,
}

/// Every decision cited under `crates/` is one the plan still holds in
/// force, reported as findings.
///
/// A citation of a dead decision is exempt when the decision's successor
/// is cited in the same comment block -- which is what `timed.rs` does,
/// and doing it is the point rather than a loophole: that comment names
/// the obsolesced decision *in order to say it is obsolesced* and to hand
/// the reader the narrower rule that replaced it. A reader who meets that
/// paragraph is not misled. One who meets a lone dead citation is.
///
/// The exemption is a test on names and not on prose, so somebody
/// determined could satisfy it by mentioning the successor without
/// thinking about what it says. This checks that the correction is
/// *present*, not that it is *right*.
///
/// Deliberately unannotated, for the reason `density.rs` gives: the
/// property it enforces is about the plan's own records rather than about
/// a rule `docs/spec/nsh` states, and claiming coverage of one from here
/// would be a lie about who implements what.
pub(crate) fn cited_decisions_are_live() -> Vec<String> {
    let workspace = workspace_root();
    let (decisions, mut reported) = corpus(&workspace);
    if decisions.is_empty() {
        /* Without this the sweep below would resolve nothing, report
         * every citation as unknown, and -- if the tree ever held no
         * citations at all -- pass while reading an empty corpus. */
        reported.push(format!(
            "no decisions were read from {CORPUS}/; this check cannot resolve \
             anything and its silence would mean nothing"
        ));
        return reported;
    }

    let mut seen = 0_usize;
    for path in files_in(&workspace, TREE) {
        /* Lossy rather than `read_to_string`, so a file that is not UTF-8
         * is still swept. Replacement characters cannot invent a citation
         * and cannot hide one, whereas skipping the file would do the
         * second. */
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                reported.push(format!(
                    "{} is not readable: {error}",
                    relative_to_workspace(&path, &workspace)
                ));
                continue;
            }
        };
        let source = String::from_utf8_lossy(&bytes);
        let found = citations(&source);
        seen += found.len();
        let beside: BTreeSet<(usize, &str)> = found
            .iter()
            .map(|citation| (citation.block, citation.id.as_str()))
            .collect();
        let name = relative_to_workspace(&path, &workspace);

        for citation in &found {
            let Citation { line, block, id } = citation;
            let Some(decision) = decisions.get(id.as_str()) else {
                reported.push(format!(
                    "{name}:{line} cites {id}, which no file in {CORPUS}/ declares; \
                     the id is misspelt, or it names a decision this repository \
                     does not keep"
                ));
                continue;
            };
            if LIVE.contains(&decision.state.as_str()) {
                continue;
            }
            if decision
                .superseded_by
                .iter()
                .any(|successor| beside.contains(&(*block, successor.as_str())))
            {
                continue;
            }
            let tail = if decision.superseded_by.is_empty() {
                "and nothing in the corpus supersedes it, so there is no successor \
                 to move to -- the reason this comment gives is not in force, and \
                 it wants rewriting or removing"
                    .to_owned()
            } else {
                format!(
                    "and it is superseded by {} -- read the successor and cite it \
                     here instead, or name it beside this citation and say what \
                     the code still stands on",
                    decision.superseded_by.join(" and ")
                )
            };
            reported.push(format!(
                "{name}:{line} cites {id}, whose state is {} rather than one of {}, \
                 {tail}",
                decision.state,
                LIVE.join(", ")
            ));
        }
    }

    if seen == 0 {
        reported.push(format!(
            "no decision citations were found under {TREE}/; either the source \
             stopped citing the plan or this check stopped reading it, and \
             neither should pass quietly"
        ));
    }
    reported
}

/// The corpus, and whatever could not be read of it.
///
/// A file that does not yield an id and a state is a finding rather than a
/// silent omission: a decision that drops out of the projection is one
/// every citation of it then resolves against nothing.
fn corpus(workspace: &Path) -> (BTreeMap<String, Decision>, Vec<String>) {
    let directory = workspace.join(CORPUS);
    let mut decisions = BTreeMap::new();
    let mut reported = Vec::new();
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            return (
                decisions,
                vec![format!("{CORPUS}/ is not readable: {error}")],
            );
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    paths.sort();

    let mut replacements = Vec::new();
    for path in paths {
        let name = relative_to_workspace(&path, workspace);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                reported.push(format!("{name} is not readable: {error}"));
                continue;
            }
        };
        match decision_in(&text) {
            Err(reason) => reported.push(format!("{name} {reason}")),
            Ok((id, state, supersedes)) => {
                for dead in supersedes {
                    replacements.push((id.clone(), dead));
                }
                let already = decisions.insert(
                    id.clone(),
                    Decision {
                        state,
                        superseded_by: Vec::new(),
                    },
                );
                if already.is_some() {
                    reported.push(format!(
                        "{name} declares {id}, which another file in {CORPUS}/ \
                         already declares"
                    ));
                }
            }
        }
    }

    for (successor, dead) in replacements {
        match decisions.get_mut(&dead) {
            Some(decision) => decision.superseded_by.push(successor),
            None => reported.push(format!(
                "{successor} supersedes {dead}, which no file in {CORPUS}/ declares"
            )),
        }
    }
    (decisions, reported)
}

/// One decision file's id, state, and the decisions it supersedes.
///
/// The frontmatter is Styx between two `---` lines. `id` and `state` are
/// top-level fields, so they are read at column zero, which is also what
/// keeps a string inside `alternatives` from being mistaken for one.
fn decision_in(text: &str) -> Result<(String, String, Vec<String>), String> {
    let frontmatter = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
        .map(|(front, _)| front)
        .ok_or("has no `---` frontmatter block")?;

    let mut id = None;
    let mut state = None;
    for line in frontmatter.lines() {
        if let Some(rest) = line.strip_prefix("id ") {
            id = citation_at(rest.trim().as_bytes(), 0);
        }
        if let Some(rest) = line.strip_prefix("state ") {
            state = Some(rest.trim().to_owned());
        }
    }
    let id = id.ok_or("has no top-level `id [dec:...]` field in its frontmatter")?;
    let state = state.ok_or("has no top-level `state @...` field in its frontmatter")?;
    if !state.starts_with('@') {
        return Err(format!("states {state:?}, which is not an `@` state name"));
    }
    Ok((id, state, superseded_ids(frontmatter)))
}

/// The ids in the frontmatter's `edges.supersedes` list.
///
/// The list is authored on one line today and the walk allows it to span
/// several, ending where the parentheses balance. A `supersedes` inside an
/// `alternatives` string would have to begin a line and be followed by an
/// open parenthesis to be mistaken for the field; nothing authors one that
/// way, and the cost of being wrong is naming a successor that is not one.
fn superseded_ids(frontmatter: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut depth = 0_usize;
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if depth == 0 {
            let Some(rest) = trimmed.strip_prefix("supersedes") else {
                continue;
            };
            if !rest.trim_start().starts_with('(') {
                continue;
            }
        }
        depth += trimmed.matches('(').count();
        depth -= trimmed.matches(')').count().min(depth);
        let bytes = trimmed.as_bytes();
        for at in 0..bytes.len() {
            if let Some(id) = citation_at(bytes, at) {
                ids.push(id);
            }
        }
        if depth == 0 {
            break;
        }
    }
    ids
}

/// Every citation in a source, with the comment block each sits in.
///
/// A block is a run of consecutive comment lines, judged by what the
/// trimmed line begins with rather than by tracking `/* */` nesting: this
/// repository carries `/*` and `*/` inside string literals -- two of these
/// checks search for them -- so a depth counter reading raw text runs away
/// and swallows the rest of a file into one block. A prefix test cannot,
/// and a block that is cut short only ever costs an exemption, which is
/// the safe direction to be wrong in.
///
/// A line that is not a comment gets a block of its own, so a citation in
/// code is beside nothing but what shares its line.
fn citations(source: &str) -> Vec<Citation> {
    let mut found = Vec::new();
    let mut block = 0_usize;
    let mut previous_was_comment = false;
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let comment = ["//", "/*", "*/", "*"]
            .iter()
            .any(|opening| trimmed.starts_with(opening));
        if !comment || !previous_was_comment {
            block += 1;
        }
        previous_was_comment = comment;

        let bytes = line.as_bytes();
        for at in 0..bytes.len() {
            if let Some(id) = citation_at(bytes, at) {
                found.push(Citation {
                    line: index + 1,
                    block,
                    id,
                });
            }
        }
    }
    found
}

/// The citation beginning at `at`, in canonical form.
///
/// Two spellings are in use and neither is the odd one out: the bare
/// bracketed id, and the same id with a backtick inside each bracket,
/// which renders as code in rustdoc. Both mean the same thing and both
/// are answered in the bare form, because a check that read one and not
/// the other would leave a place to put a citation where nobody looks
/// for one. Neither is written out here; see the note at the top of the
/// file.
fn citation_at(bytes: &[u8], at: usize) -> Option<String> {
    let mut cursor = at;
    if bytes.get(cursor) != Some(&b'[') {
        return None;
    }
    cursor += 1;
    let backticked = bytes.get(cursor) == Some(&b'`');
    if backticked {
        cursor += 1;
    }
    if !bytes[cursor..].starts_with(b"dec:") {
        return None;
    }
    let start = cursor;
    while bytes.get(cursor).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.')
    }) {
        cursor += 1;
    }
    let id = std::str::from_utf8(&bytes[start..cursor]).ok()?;
    if backticked {
        if bytes.get(cursor) != Some(&b'`') {
            return None;
        }
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b']') {
        return None;
    }
    /* `dec`, a corpus prefix, and a name: anything else is a bracketed
     * word that begins with the same four bytes rather than an id. */
    let parts: Vec<&str> = id.split(':').collect();
    (parts.len() == 3 && parts.iter().all(|part| !part.is_empty())).then(|| format!("[{id}]"))
}

/// Every file under `tree`, whatever its extension.
///
/// Not `rust_sources_in`: a citation in a manifest comment or in one of
/// the checked-in registers under `crates/` goes stale exactly as one in a
/// doc comment does, and a check that reads only `.rs` would be an
/// invitation to move a dead reason one file sideways.
fn files_in(workspace: &Path, tree: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    files_below(&workspace.join(tree), &mut files);
    files.sort();
    files
}

fn files_below(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("source entry is readable").path();
        if path.is_dir() {
            files_below(&path, files);
        } else {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `[dec` of a fabricated id, kept apart from its tail so that no
    /// test in this file spells a citation the sweep would then find in
    /// the checker's own source.
    const OPEN: &str = "[dec";

    /// Both spellings are citations, and neither the brackets nor the
    /// backticks reach the answer.
    #[test]
    fn a_backticked_citation_is_the_same_citation() {
        let plain = format!("//! see {OPEN}:nsh:a-name].");
        let coded = format!("/// see {OPEN}:nsh:a-name`], which");
        let coded = coded.replace("[dec", "[`dec");
        let ids = |text: &str| {
            citations(text)
                .into_iter()
                .map(|citation| citation.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&plain), vec![format!("{OPEN}:nsh:a-name]")]);
        assert_eq!(ids(&coded), vec![format!("{OPEN}:nsh:a-name]")]);
    }

    /// The four bytes are not enough on their own, and a mismatched
    /// backtick is not a citation either.
    #[test]
    fn a_bracketed_word_is_not_a_citation() {
        for text in [
            format!("const NEEDLE: &str = \"{OPEN}:\";"),
            format!("//! a {OPEN}:nsh] with two parts"),
            format!("//! a {OPEN}::name] with an empty one"),
            format!("//! an unclosed {OPEN}:nsh:name"),
            format!("//! a half-quoted [`{}:nsh:name]", &OPEN[1..]),
        ] {
            assert!(citations(&text).is_empty(), "{text} was read as a citation");
        }
    }

    /// A run of comment lines is one block and code between them is not,
    /// which is what decides whether a correction counts as beside a dead
    /// citation.
    #[test]
    fn a_comment_run_is_one_block() {
        let source = format!(
            "//! {OPEN}:nsh:one]\n\
             //! and {OPEN}:nsh:two]\n\
             \n\
             /* {OPEN}:nsh:three]\n\
             \x20* and {OPEN}:nsh:four] */\n\
             let x = 1; // {OPEN}:nsh:five]\n"
        );
        let blocks: Vec<usize> = citations(&source)
            .into_iter()
            .map(|citation| citation.block)
            .collect();
        assert_eq!(blocks[0], blocks[1], "one doc comment is one block");
        assert_eq!(blocks[2], blocks[3], "one block comment is one block");
        assert_ne!(blocks[1], blocks[2], "a blank line ends a block");
        assert_ne!(blocks[3], blocks[4], "a line of code is its own block");
    }

    /// The corpus this repository keeps is read, states and all -- the
    /// property that would break silently if the frontmatter's shape
    /// changed under the parser above.
    #[test]
    fn the_corpus_reads() {
        let workspace = workspace_root();
        let (decisions, reported) = corpus(&workspace);
        assert!(reported.is_empty(), "{reported:?}");
        assert!(
            decisions.len() >= 20,
            "{} decisions read, which is fewer than the corpus holds",
            decisions.len()
        );
        assert!(
            decisions
                .values()
                .any(|decision| decision.state == "@decided"),
            "no decision read as @decided"
        );
        assert!(
            decisions
                .values()
                .any(|decision| !decision.superseded_by.is_empty()),
            "no supersedes edge was read, so no finding could ever name a successor"
        );
    }
}
