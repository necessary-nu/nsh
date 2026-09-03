//! Filename completion, as the shell answers it.
//!
//! nshedit asks for the candidates that extend a stem and has no opinion
//! about what a candidate is; that is host policy, and this is the host.
//! The stem is split at its last path separator, the directory named by
//! the prefix is read, the entries that extend the basename are kept, and
//! each is marked the way the shell marks it -- `/` after a directory, a
//! space after anything else.
//!
//! The stem-shaped entry point is separate from the one nshedit calls
//! because `display_expansions` and `expand_all` ask the same question
//! about a token taken from the line rather than about a completion query.

use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};
use nshedit::domain::{Text, TextUnit};
use nshedit::editor::CompletionCandidate;

use super::{text_from_bytes, text_to_bytes};

// [spec:posix:req:edit.command-complete-unique]
pub(super) fn completion_candidates(
    query: &nshedit::editor::CompletionQuery,
) -> nshedit::editor::CompletionCandidates {
    completion_candidates_for_stem(query.stem())
}

pub(super) fn completion_candidates_for_stem(stem: &Text) -> nshedit::editor::CompletionCandidates {
    let Ok(stem) = text_to_bytes(stem) else {
        return Vec::new().into();
    };
    let split = nsh_platform::shell_path_last_separator(&stem)
        .map_or((b"".as_slice(), stem.as_slice()), |position| {
            (&stem[..=position], &stem[position + 1..])
        });
    let (prefix, basename) = split;
    let directory = if prefix.is_empty() {
        b".".as_slice()
    } else {
        prefix
    };
    let Ok(directory) = directory.try_to_path_buf() else {
        return Vec::new().into();
    };
    let Ok(entries) = nsh_platform::read_directory(&directory) else {
        return Vec::new().into();
    };
    let mut candidates = entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.name.to_shell_bytes();
            if !name.starts_with(basename) {
                return None;
            }
            let mut insertion = prefix.to_vec();
            insertion.extend_from_slice(&name);
            let suffix = if entry.is_directory { "/" } else { " " };
            Some(CompletionCandidate::new(text_from_bytes(&insertion)).with_suffix(suffix))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.insertion().cmp(right.insertion()));
    candidates.into()
}

pub(super) fn all_completion_insertions(
    candidates: &nshedit::editor::CompletionCandidates,
) -> Text {
    let mut expansion = Text::default();
    for candidate in candidates.iter() {
        if !expansion.is_empty() {
            expansion.push(TextUnit::Scalar(' '));
        }
        expansion.extend(candidate.insertion().as_units().iter().copied());
    }
    expansion
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn completion_marks_files_and_directories() {
        /* Every mode below comes from the process's file-creation mask,
         * which `builtins::umask`'s tests drive to 0o777 while they run,
         * and an explicit mode cannot escape it: the kernel applies
         * `mode & ~umask` to each `mkdir` and `open`, so a
         * `DirBuilder::mode(0o700)` under a 0o777 mask arrives 0o000 and
         * the first write into the directory is EACCES. The mask has to
         * be excluded rather than overridden. Unlocked, this failed 201
         * times in 2,000 runs at `--test-threads 8`. */
        let _guard = crate::test_support::lock();
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "nsh-completion-test-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("alpha-file"), []).unwrap();
        std::fs::create_dir(directory.join("alpha-directory")).unwrap();
        std::fs::write(directory.join("beta"), []).unwrap();

        let mut stem = directory.to_shell_bytes();
        stem.extend_from_slice(b"/alpha");
        let candidates = completion_candidates_for_stem(&text_from_bytes(&stem));
        assert_eq!(candidates.len(), 2);
        let mut suffixes = candidates
            .iter()
            .map(|candidate| {
                (
                    text_to_bytes(candidate.insertion()).unwrap(),
                    candidate.suffix().cloned(),
                )
            })
            .collect::<Vec<_>>();
        suffixes.sort_by(|left, right| left.0.cmp(&right.0));
        assert!(suffixes[0].0.ends_with(b"alpha-directory"));
        assert_eq!(suffixes[0].1, Some(Text::from("/")));
        assert!(suffixes[1].0.ends_with(b"alpha-file"));
        assert_eq!(suffixes[1].1, Some(Text::from(" ")));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn all_insertions_have_single_spaces() {
        let candidates = vec![
            CompletionCandidate::new("alpha1").with_suffix(" "),
            CompletionCandidate::new("alpha2").with_suffix("/"),
        ]
        .into();
        assert_eq!(
            all_completion_insertions(&candidates),
            Text::from("alpha1 alpha2")
        );
    }
}
