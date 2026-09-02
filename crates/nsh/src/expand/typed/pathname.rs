//! Pathname expansion over quote-aware typed patterns.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice as _};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use super::Field;
use crate::context::Shell;
use crate::error::Error;
use crate::options::{BashShopt, Dialect};
use crate::pattern::{Pattern, PatternOptions};

/// One directory reached so far, with the pathname the shell will show
/// for it. The two differ whenever the pattern is relative: the walk
/// starts at `.` but `.` is not part of any generated word.
struct Candidate {
    path: PathBuf,
    display: BString,
}

/// The shell state one pathname expansion reads, gathered once.
///
/// Bash's glob behaviour is spread over five `shopt` names and one
/// variable; collecting them keeps the traversal free of shell lookups.
/// The POSIX case is not simply every bit off, because the two
/// references disagree about the text of a pattern as well as about its
/// options: dash copies a repeated slash through and Bash does not.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) struct GlobSettings {
    options: PatternOptions,
    dot_names: bool,
    /// Bash's `globskipdots`, which is on unless a script turns it off
    /// and then makes `.` and `..` names no pattern can match. POSIX
    /// mode has no such option and matches them, as dash does.
    skip_dots: bool,
    globstar: bool,
    nullglob: bool,
    failglob: bool,
    /// Whether a run of slashes narrows to one once a component has had
    /// to be matched. Bash copies the pattern's own text through only
    /// while it is still reading literal text -- `a//*` is `a//b`, and
    /// `*//*` is `a/b` -- where dash copies every run through, so this
    /// is off in POSIX mode.
    collapse_after_match: bool,
    /// `GLOBIGNORE`, already split into patterns. A non-empty list also
    /// reveals dot names, and hides any word whose last component is `.`
    /// or `..` however the pattern spelled it.
    ignored: Vec<Pattern>,
}

impl GlobSettings {
    /// Whether `.` and `..` are names a component can match at all.
    ///
    /// They are not directory entries here -- the platform's read does
    /// not report them -- so a pattern that could match them has them
    /// added back, and this is the question of whether to add them.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn skips_dot_entries(&self) -> bool {
        self.skip_dots || !self.ignored.is_empty()
    }

    /// The slashes to write in front of one component, given whether
    /// anything before it had to be matched.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn separator(&self, saw_meta: bool, slashes: usize) -> usize {
        if self.collapse_after_match && saw_meta {
            1
        } else {
            slashes
        }
    }
}

// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn settings(shell: &mut Shell) -> GlobSettings {
    if shell.options.dialect() != Dialect::Bash {
        return GlobSettings {
            options: PatternOptions::NONE,
            dot_names: false,
            skip_dots: false,
            globstar: false,
            nullglob: false,
            failglob: false,
            collapse_after_match: false,
            ignored: Vec::new(),
        };
    }
    let options = PatternOptions {
        extended: shell.options.shopt(BashShopt::ExtGlob),
        ignore_case: shell.options.shopt(BashShopt::NoCaseGlob),
    };
    let ignored = crate::variables::lookup_bytes(shell, BStr::new(b"GLOBIGNORE"))
        .filter(|value| !value.is_empty())
        .map(|value| {
            ignore_patterns(value.as_bstr())
                .into_iter()
                .map(|text| Pattern::from_escaped_text(text, options))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    GlobSettings {
        options,
        dot_names: !ignored.is_empty() || shell.options.shopt(BashShopt::DotGlob),
        skip_dots: shell.options.shopt(BashShopt::GlobSkipDots),
        globstar: shell.options.shopt(BashShopt::GlobStar),
        nullglob: shell.options.shopt(BashShopt::NullGlob),
        failglob: shell.options.shopt(BashShopt::FailGlob),
        collapse_after_match: true,
        ignored,
    }
}

/// Split `GLOBIGNORE` into its patterns.
///
/// The separator is a colon, but a colon inside a bracket expression is
/// part of the pattern: that is what makes `GLOBIGNORE=[[:alnum:]]*` one
/// pattern rather than the three that a plain split would produce. A
/// backslash protects the byte after it, as it does everywhere else in
/// the value.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn ignore_patterns(value: &BStr) -> Vec<&[u8]> {
    let bytes: &[u8] = value.as_ref();
    let mut patterns = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'[' => at = bracket_end(bytes, at).unwrap_or(at + 1),
            b':' => {
                patterns.push(&bytes[start..at]);
                at += 1;
                start = at;
            }
            _ => at += 1,
        }
    }
    patterns.push(&bytes[start..]);
    patterns
}

/// Where a bracket expression that opens at `at` ends, or `None` when it
/// never closes and the `[` is therefore ordinary text.
// [spec:posix:syn:pattern.bracket-expression]
fn bracket_end(bytes: &[u8], at: usize) -> Option<usize> {
    let mut cursor = at + 1;
    if matches!(bytes.get(cursor), Some(b'!' | b'^')) {
        cursor += 1;
    }
    /* A `]` in the first member position is the character itself. */
    if bytes.get(cursor) == Some(&b']') {
        cursor += 1;
    }
    while cursor < bytes.len() {
        if bytes[cursor] == b']' {
            return Some(cursor + 1);
        }
        cursor = match bytes.get(cursor + 1) {
            Some(&delimiter @ (b':' | b'.' | b'=')) if bytes[cursor] == b'[' => {
                inner_member_end(bytes, cursor + 2, delimiter)?
            }
            _ => cursor + 1,
        };
    }
    None
}

/// Where a `[:class:]`, `[.collating.]` or `[=equivalence=]` member that
/// starts at `from` ends.
fn inner_member_end(bytes: &[u8], from: usize, delimiter: u8) -> Option<usize> {
    let mut cursor = from;
    while cursor + 1 < bytes.len() {
        if bytes[cursor] == delimiter && bytes[cursor + 1] == b']' {
            return Some(cursor + 2);
        }
        cursor += 1;
    }
    None
}

// Quote protection is part of `Pattern`, candidates are owned paths, and
// sorting uses the shell locale. These operations collectively replace the
// cursor-based glob, escape-counting, and linked-list merge-sort helpers.
// [spec:posix:req:expand.pathname]
// [spec:posix:def:pattern.filename-expansion-qualification]
// [spec:posix:req:pattern.filename-expansion-trigger]
// [spec:posix:req:pattern.no-special-chars-unchanged]
// [spec:posix:req:pattern.no-match-unchanged]
// [spec:posix:req:pattern.replacement-sorted]
// [spec:dash:sem:expand.addfname-common-fn]
// [spec:dash:sem:expand.addfnamealt-fn]
// [spec:dash:sem:expand.addglob-fn]
// [spec:dash:sem:expand.esclen-fn]
// [spec:dash:sem:expand.expandmeta-fn]
// [spec:dash:sem:expand.expandmeta-glob-fn]
// [spec:dash:sem:expand.expmeta-fn]
// [spec:dash:sem:expand.expmeta-rmescapes-fn]
// [spec:dash:sem:expand.expsort-fn]
// [spec:dash:sem:expand.mesclen-fn]
// [spec:dash:sem:expand.msort-fn]
// [spec:dash:sem:expand.opendir-interruptible-fn]
// [spec:dash:sem:expand.preglob-fn]
pub(super) fn expand(
    shell: &mut Shell,
    fields: Vec<Field>,
    settings: &GlobSettings,
) -> Result<Vec<Field>, Error> {
    let mut expanded = Vec::with_capacity(fields.len());
    for field in fields {
        let pattern = field.pattern(settings.options);
        if !pattern.has_meta() {
            expanded.push(field);
            continue;
        }
        let mut names = matches(&shell.locale, &pattern, settings);
        names.sort_by(|left, right| shell.locale.collate(left, right));
        names.dedup();
        if names.is_empty() {
            if settings.failglob {
                return Err(no_match_error(shell, &pattern));
            }
            if !settings.nullglob {
                expanded.push(field);
            }
            continue;
        }
        expanded.extend(
            names
                .into_iter()
                .map(|bytes| Field::from_bytes(&bytes, false, false, false)),
        );
    }
    Ok(expanded)
}

// [spec:nsh:req:compat.bash.expansion-globbing]
/// `failglob` reports the pattern that matched nothing. The failure is
/// an expansion failure: the command it belongs to never runs, and its
/// status is the failure status rather than a diagnostic status.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn no_match_error(shell: &mut Shell, pattern: &Pattern) -> Error {
    let mut message = b"no match: ".to_vec();
    message.extend_from_slice(pattern.as_bytes());
    /* `failglob` is a Bash option, so its refusal takes the Bash boundary:
     * the command the pattern belongs to does not run, and the shell moves
     * to the next record rather than ending. Reaching for the expansion
     * error here instead would make one unmatched glob fatal, which is the
     * behaviour `[spec:nsh:req:compat.bash.error-boundary]` exists to end. */
    shell.diagnostics().dialect_error(&message)
}

// [spec:posix:req:pattern.leading-period]
// [spec:posix:req:pattern.leading-period-in-bracket-unspecified]
// [spec:posix:req:pattern.directory-permissions]
// [spec:posix:req:pattern.permission-errors-not-fatal]
fn matches(
    locale: &nsh_platform::Locale,
    pattern: &Pattern,
    settings: &GlobSettings,
) -> Vec<BString> {
    let split = split(pattern);
    let absolute = split.leading > 0;
    let mut candidates = vec![Candidate {
        path: PathBuf::from(if absolute { "/" } else { "." }),
        display: slashes(split.leading),
    }];
    let mut saw_meta = false;

    for (index, step) in split.steps.iter().enumerate() {
        let separator = settings.separator(saw_meta, step.slashes);
        let component = &step.component;
        if settings.globstar && is_globstar(component) {
            saw_meta = true;
            let next = split.steps.get(index + 1);
            /* The word ends at this `**` when nothing follows it, and
             * Bash ends it at a repeated slash too. A `**` written with
             * two slashes and then a name generates what the same `**`
             * with one slash and no name generates, and looks for that
             * name in each: the directory the walk starts from has to be
             * a directory and is its own word, rather than only a place
             * to carry on from. */
            let ends_word = next.is_none();
            let ends_here = next.is_none_or(|next| next.slashes > 1);
            let start = if !ends_here {
                StartMatch::Prefix
            } else if split.steps[..index]
                .iter()
                .any(|earlier| earlier.component.has_meta())
            {
                StartMatch::Matched
            } else {
                StartMatch::Literal
            };
            let walk = Walk {
                directories_only: !ends_word || split.trailing > 0,
                start,
                separator,
                /* Bash reads a pattern that opens with a `**` component
                 * by walking the tree itself, and every other `**` by
                 * expanding what precedes it into words and matching the
                 * rest inside them, which resolves a link the way any
                 * other component does. A leading `**` therefore stops
                 * at a link to a directory and a later one carries the
                 * word on through it. */
                enter_links: split.steps[..index]
                    .iter()
                    .any(|earlier| !is_globstar(&earlier.component)),
                settings,
            };
            candidates = walk.descendants(candidates);
            continue;
        }
        if component.has_meta() {
            saw_meta = true;
            candidates = children(locale, candidates, separator, component, settings);
        } else {
            let Ok(name) = component.as_bytes().try_to_os_string() else {
                return Vec::new();
            };
            for candidate in &mut candidates {
                candidate.path = candidate.path.join(&name);
                candidate.display =
                    append_component(&candidate.display, separator, component.as_bytes());
            }
        }
    }
    let trailing = settings.separator(saw_meta, split.trailing);

    if !saw_meta {
        return Vec::new();
    }
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let exists = if split.trailing > 0 {
                nsh_platform::path_metadata(&candidate.path, true)
                    .is_ok_and(|metadata| metadata.kind == nsh_platform::FileKind::Directory)
            } else {
                nsh_platform::path_metadata(&candidate.path, false).is_ok()
            };
            let mut display = candidate.display;
            if split.trailing > 0 && display.last() != Some(&b'/') {
                display.extend_from_slice(&slashes(trailing));
            }
            (exists && !is_ignored(locale, display.as_ref(), settings)).then_some(display)
        })
        .collect()
}

/// Whether this component is the bare `**` that `globstar` gives its own
/// meaning. Adjacent literal bytes, or quoting, leave it an ordinary
/// pattern in which `**` is just `*`.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn is_globstar(component: &Pattern) -> bool {
    component.as_bytes() == b"**" && component.quote_bits().iter().all(|quoted| !quoted)
}

/// How the directory a `**` starts from is written into the words that
/// `**` generates.
///
/// Bash matches the last component of a pattern against the directory
/// everything before it names, and `**` matches that directory itself.
/// The word it produces for it is the prefix Bash already had in hand:
/// text the pattern spelled out comes back verbatim, slash and all, so
/// `a/**` yields `a/`, while a prefix the shell had to match contributes
/// only the path it found, so `?/**` yields `a`. A bare `**` has no
/// prefix at all, and that is the field it does not generate.
// [spec:nsh:req:compat.bash.expansion-globbing]
#[derive(Clone, Copy, PartialEq, Eq)]
enum StartMatch {
    /// The word continues past this `**`, so the directory is only
    /// somewhere to carry on from and even an empty prefix is one.
    Prefix,
    /// The word ends here and the prefix was matched, not spelled out.
    Matched,
    /// The word ends here and the prefix is the pattern's own text.
    Literal,
}

/// The part of one `**` that does not change as the walk goes down: what
/// it is allowed to match, and how the directory it starts from is
/// written into the words it generates.
struct Walk<'a> {
    directories_only: bool,
    start: StartMatch,
    /// The slashes the pattern wrote between the prefix and this `**`,
    /// which is what separates the prefix from the first level of the
    /// walk. Every level below that is the walk's own, so it writes one.
    separator: usize,
    /// Whether a link to a directory is a place this `**` carries the
    /// word on from.
    enter_links: bool,
    settings: &'a GlobSettings,
}

impl Walk<'_> {
    /// Every directory reachable from these candidates, the candidates
    /// themselves included, plus their non-directory entries when `**`
    /// ends the pattern.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn descendants(&self, candidates: Vec<Candidate>) -> Vec<Candidate> {
        let mut result = Vec::new();
        let mut pending = Vec::new();
        for candidate in candidates {
            self.entries_below(&candidate, self.separator, &mut pending, &mut result);
            if let Some(candidate) = self.starting_match(candidate) {
                result.push(candidate);
            }
        }
        /* Everything the walk finds for itself is a directory it reached
         * by name, so it is its own word as well as somewhere to go on
         * from. Only the candidates above were reached some other way. */
        while let Some(candidate) = pending.pop() {
            self.entries_below(&candidate, 1, &mut pending, &mut result);
            result.push(candidate);
        }
        result
    }

    /// Sort one directory's entries into the ones the walk descends into
    /// and the ones it generates a word for.
    ///
    /// Descent is over the directory tree only, so a symbolic link is
    /// never a way in — that is why `**` reaches a link to a directory
    /// but nothing under it.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn entries_below(
        &self,
        candidate: &Candidate,
        separator: usize,
        pending: &mut Vec<Candidate>,
        result: &mut Vec<Candidate>,
    ) {
        for entry in nsh_platform::read_directory(&candidate.path).unwrap_or_default() {
            let bytes = entry.name.to_shell_bytes();
            if bytes.first() == Some(&b'.') && !self.settings.dot_names {
                continue;
            }
            let child = Candidate {
                path: candidate.path.join(&entry.name),
                display: append_component(&candidate.display, separator, &bytes),
            };
            if entry.is_directory {
                pending.push(child);
            } else if !self.directories_only || (self.reaches_links() && is_directory(&child.path))
            {
                result.push(child);
            }
        }
    }

    /// Whether a name that resolves to a directory without being one is a
    /// word this `**` generates.
    ///
    /// A `**` the word ends at asks what a name resolves to, so a link to
    /// a directory is one. A `**` the word carries on past produces the
    /// places the rest of the word is matched in, and Bash matches inside
    /// a link there for every `**` except one the pattern opens with.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn reaches_links(&self) -> bool {
        self.start != StartMatch::Prefix || self.enter_links
    }

    /// The word this `**` generates for the directory it starts from, or
    /// `None` where Bash generates none: `**` matches a directory and not
    /// a file, and a prefix that is nothing at all is not a field.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn starting_match(&self, mut candidate: Candidate) -> Option<Candidate> {
        if self.start == StartMatch::Prefix {
            return Some(candidate);
        }
        if candidate.display.is_empty() || !is_directory(&candidate.path) {
            return None;
        }
        if self.start == StartMatch::Literal && candidate.display.last() != Some(&b'/') {
            candidate
                .display
                .extend_from_slice(&slashes(self.separator.max(1)));
        }
        Some(candidate)
    }
}

/// Whether the path names a directory, a symbolic link to one included.
fn is_directory(path: &Path) -> bool {
    nsh_platform::path_metadata(path, true)
        .is_ok_and(|metadata| metadata.kind == nsh_platform::FileKind::Directory)
}

/// The entries of each candidate directory that one pattern component
/// matches.
fn children(
    locale: &nsh_platform::Locale,
    candidates: Vec<Candidate>,
    separator: usize,
    component: &Pattern,
    settings: &GlobSettings,
) -> Vec<Candidate> {
    let literal_dot = component.starts_with_literal_dot();
    let mut result = Vec::new();
    for candidate in candidates {
        let Ok(entries) = nsh_platform::read_directory(&candidate.path) else {
            continue;
        };
        let mut names = entries
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        if literal_dot && !settings.skips_dot_entries() {
            names.push(OsString::from("."));
            names.push(OsString::from(".."));
        }
        for name in names {
            let bytes = name.to_shell_bytes();
            if bytes.first() == Some(&b'.') && !literal_dot && !settings.dot_names {
                continue;
            }
            if component.matches(locale, &bytes) {
                result.push(Candidate {
                    path: candidate.path.join(&name),
                    display: append_component(&candidate.display, separator, &bytes),
                });
            }
        }
    }
    result
}

/// Whether `GLOBIGNORE` hides this generated pathname.
///
/// Each pattern is matched one pathname component at a time, so `*` and
/// `?` never cross a `/` — `*.txt` hides `one.txt` but not `foo/two.txt`.
///
/// A non-empty list also hides `.` and `..` outright, and it hides them
/// however the pattern spelled them: `*/.` matches nothing under any
/// `GLOBIGNORE` at all, where `*/./f` still matches. That is the same
/// rule `globskipdots` states about a directory's entries, reaching a
/// component the walk never had to read a directory for.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn is_ignored(locale: &nsh_platform::Locale, display: &BStr, settings: &GlobSettings) -> bool {
    if settings.ignored.is_empty() {
        return false;
    }
    let last = display
        .rsplit(|byte| *byte == b'/')
        .next()
        .unwrap_or_default();
    if last == b"." || last == b".." {
        return true;
    }
    settings
        .ignored
        .iter()
        .any(|ignored| matches_by_component(locale, ignored, display))
}

fn matches_by_component(locale: &nsh_platform::Locale, ignored: &Pattern, display: &BStr) -> bool {
    let parts = components(ignored);
    let mut names = display.split(|byte| *byte == b'/');
    for part in &parts {
        let Some(name) = names.next() else {
            return false;
        };
        if !part.matches(locale, name) {
            return false;
        }
    }
    names.next().is_none()
}

// [spec:posix:req:pattern.slash-explicit-match]
// [spec:posix:syn:pattern.slash-terminates-bracket]
fn components(pattern: &Pattern) -> Vec<Pattern> {
    let mut components = Vec::new();
    let mut start = 0;
    for (at, byte) in pattern.as_bytes().iter().enumerate() {
        if *byte == b'/' {
            components.push(pattern.slice(start..at));
            start = at + 1;
        }
    }
    components.push(pattern.slice(start..pattern.as_bytes().len()));
    components
}

/// One component of a pattern with the run of slashes written in front
/// of it, which is part of the word and not only a separator.
struct Step {
    /// How many slashes the pattern spells between the component before
    /// this one and this one. Zero for the first component.
    slashes: usize,
    component: Pattern,
}

/// A pattern cut into the components a walk visits, keeping the width of
/// every slash run.
///
/// [`components`] answers a different question and drops that width: it
/// cuts a `GLOBIGNORE` entry into the parts one pathname is matched
/// against, where an empty part is a part that matches nothing.
struct Split {
    /// The slashes the pattern opens with, which are its root when there
    /// are any and nothing when there are none.
    leading: usize,
    steps: Vec<Step>,
    /// The slashes the pattern ends with.
    trailing: usize,
}

// [spec:posix:req:pattern.slash-explicit-match]
fn split(pattern: &Pattern) -> Split {
    let bytes = pattern.as_bytes();
    let mut split = Split {
        leading: 0,
        steps: Vec::new(),
        trailing: 0,
    };
    let mut run = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'/' {
            run += 1;
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && bytes[at] != b'/' {
            at += 1;
        }
        if split.steps.is_empty() {
            split.leading = run;
            run = 0;
        }
        split.steps.push(Step {
            slashes: run,
            component: pattern.slice(start..at),
        });
        run = 0;
    }
    split.trailing = run;
    split
}

fn slashes(count: usize) -> BString {
    BString::from(vec![b'/'; count])
}

fn append_component(prefix: &[u8], separator: usize, component: &[u8]) -> BString {
    let mut result = BString::from(prefix);
    if !result.is_empty() && result.last() != Some(&b'/') {
        result.extend_from_slice(&slashes(separator.max(1)));
    }
    result.extend_from_slice(component);
    result
}
