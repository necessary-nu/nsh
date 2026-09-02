//! The cases a group is expected to fail, kept as a list of ids.
//!
//! `run-oils` ends in a summary line, and for a week the `fail=` count in
//! it was how a change was checked against the Bash comparison survey. A
//! count cannot tell a case that was fixed from one that was broken, and
//! two `process-sub` cases move between runs under load -- one of them
//! toward a *pass* -- so the number drifts for reasons no register
//! describes. One commit credited two fixes that were not real because it
//! read the count.
//!
//! Comparing the *list* of failing ids answers what the count cannot.
//! Doing that comparison in the shell does not. The extraction adopted on
//! 2026-09-01 was
//!
//! ```text
//! grep -aoE '^FAIL +[a-z0-9-]+\.test\.sh:[0-9]+' | awk '{print $2}' | sort
//! ```
//!
//! and `[a-z0-9-]` has no underscore, so `case_.test.sh:1`,
//! `command_.test.sh:3` and `command_.test.sh:12` never reached the list.
//! Every "the list is identical" claim made that week was checked on 473
//! of 476 cases and stated as though it were all of them. Nothing was in
//! fact missed -- the three are stable -- but a regression in either spec
//! would have passed the check in silence, and both specs are ones the
//! command decomposition had just rewritten.
//!
//! So the runner emits the ids itself and compares them itself. There is
//! no pattern left to get wrong, and the baseline is a file the
//! repository keeps rather than a `/tmp` artefact rebuilt by hand.
//!
//! THE PINS ARE THE OTHER HALF. A list of failing ids says nothing apart
//! from the run that produced it, so the file records the group, the
//! expectation namespace, POSIX mode, the per-case timeout and the Oils
//! commit, and a run that does not match all five is refused rather than
//! compared. The shell's own hash is deliberately not among them: it
//! changes with every build, and pinning it would make the comparison
//! refuse the only runs anyone wants to make.
//!
//! A TIMEOUT OR A HARNESS ERROR IS NEVER EXPECTED, and neither is a
//! failure, so a case that stops passing by timing out would not enter
//! the failing set and would leave the comparison silent. They are
//! reported separately and count against the verdict, which is the same
//! judgement `bash_gate` makes for the same reason.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::{Result, RunReport};

const SCHEMA: u32 = 1;

/// Prose the generator owns, so that re-recording the list does not
/// silently drop the explanation of what the list is for.
///
/// The command it quotes is the one that reproduces *this* file, group
/// and path included. A header naming some other run is how a file ends
/// up being maintained by a command that does not maintain it.
fn header(group: &str, expectation_shell: &str, path: &Path) -> String {
    let path = path.display();
    format!(
        "\
# The cases the {group} survey is expected to fail, by id.
#
# Generated. Do not edit the list by hand: re-record it with
#
#     nsh-survey run-oils --shell target/gate/bash --expect-shell {expectation_shell} \\
#       --group {group} --baseline {path} --update-baseline
#
# and read the difference the refresh prints before you keep it. Drop
# `--update-baseline` to compare instead: that exits non-zero when the sets
# differ and names every id on either side of the difference.
#
# Nothing is extracted from the run's text, which is the whole point of the
# file. The `grep -aoE '^FAIL +[a-z0-9-]+\\.test\\.sh:[0-9]+'` this replaces
# had no underscore in its character class, so `case_.test.sh:1`,
# `command_.test.sh:3` and `command_.test.sh:12` sat silently outside every
# comparison made with it.
#
# The pins are what make the list mean something. A run whose group,
# expectation namespace, POSIX mode, per-case timeout or Oils commit differs
# from these is refused rather than compared, and a filtered run (`--spec`,
# `--case`, `--max-cases`) is refused outright because every unselected case
# would read as fixed. The shell's hash is not pinned; it changes with every
# build.

"
    )
}

/// A recorded failing-case list, with the pins that make it mean
/// something.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Baseline {
    schema: u32,
    group: String,
    expectation_shell: String,
    posix: bool,
    timeout_ms: u64,
    oils_commit: String,
    failing: BTreeSet<String>,
}

impl Baseline {
    fn from_run(report: &RunReport) -> Self {
        Self {
            schema: SCHEMA,
            group: report.group.clone(),
            expectation_shell: report.expectation_shell.clone(),
            posix: report.posix,
            timeout_ms: report.timeout_ms,
            oils_commit: report.source_commit.clone(),
            failing: report.failing_ids(),
        }
    }

    fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|error| {
            format!(
                "cannot read the failing-case baseline {}: {error}",
                path.display()
            )
        })?;
        let baseline: Self =
            toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        if baseline.schema != SCHEMA {
            return Err(format!(
                "{} has unsupported schema {}",
                path.display(),
                baseline.schema
            )
            .into());
        }
        Ok(baseline)
    }

    /// Why this list does not describe that run, if it does not.
    ///
    /// Every pin here changes which cases fail, so comparing across one
    /// of them produces a difference that is about the run rather than
    /// about the shell -- the failure mode this whole file exists to
    /// stop, wearing a different hat.
    pub(super) fn mismatch(&self, report: &RunReport) -> Option<String> {
        [
            ("group", self.group.clone(), report.group.clone()),
            (
                "expectation shell",
                self.expectation_shell.clone(),
                report.expectation_shell.clone(),
            ),
            (
                "POSIX mode",
                self.posix.to_string(),
                report.posix.to_string(),
            ),
            (
                "per-case timeout",
                self.timeout_ms.to_string(),
                report.timeout_ms.to_string(),
            ),
            (
                "Oils commit",
                self.oils_commit.clone(),
                report.source_commit.clone(),
            ),
        ]
        .into_iter()
        .find(|(_, recorded, observed)| recorded != observed)
        .map(|(what, recorded, observed)| {
            format!("recorded {what} is {recorded}, this run has {observed}")
        })
    }

    fn write(&self, path: &Path) -> Result<()> {
        let header = header(&self.group, &self.expectation_shell, path);
        let body = toml::to_string_pretty(self)?;
        fs::write(path, format!("{header}{body}"))?;
        Ok(())
    }
}

/// What one run says about a recorded list.
pub(super) struct Comparison {
    newly_failing: Vec<String>,
    no_longer_failing: Vec<String>,
    unknown: Vec<String>,
    unstable: Vec<String>,
    recorded: usize,
    observed: usize,
}

impl Comparison {
    pub(super) fn of(baseline: &Baseline, report: &RunReport) -> Self {
        let observed = report.failing_ids();
        let known = report.all_ids();
        Self {
            newly_failing: observed.difference(&baseline.failing).cloned().collect(),
            /* An id the group no longer contains has not been fixed, and
             * saying it stopped failing would read as though it had. A
             * corpus bump that drops a spec is the ordinary way to get
             * one, and it should cost a re-record, not a shrug. */
            no_longer_failing: baseline
                .failing
                .difference(&observed)
                .filter(|id| known.contains(*id))
                .cloned()
                .collect(),
            unknown: baseline.failing.difference(&known).cloned().collect(),
            unstable: report.unstable_ids().into_iter().collect(),
            recorded: baseline.failing.len(),
            observed: observed.len(),
        }
    }

    pub(super) fn agrees(&self) -> bool {
        self.newly_failing.is_empty()
            && self.no_longer_failing.is_empty()
            && self.unknown.is_empty()
            && self.unstable.is_empty()
    }

    /// Report to stderr, deliberately.
    ///
    /// The verdict is about the run rather than part of its output, and
    /// stdout already carries a whole document under `--format json` and
    /// a machine-readable list under `--format ids`. Writing the verdict
    /// there would corrupt both, and a check nobody can pipe is a check
    /// people stop running.
    fn write_text(&self, path: &Path) {
        eprintln!("failing-case baseline: {}", path.display());
        eprintln!(
            "recorded {} failing cases, this run has {}",
            self.recorded, self.observed
        );
        for id in &self.newly_failing {
            eprintln!("  + {id} failed and is not in the baseline");
        }
        for id in &self.no_longer_failing {
            eprintln!("  - {id} is in the baseline and did not fail");
        }
        for id in &self.unknown {
            eprintln!("  ? {id} is in the baseline and names no case in the group");
        }
        for id in &self.unstable {
            eprintln!("  ! {id} timed out or ended in a harness error, which is never expected");
        }
        if self.agrees() {
            eprintln!("baseline: matched on all {} failing cases", self.recorded);
        } else {
            eprintln!(
                "baseline: MISMATCHED -- {} newly failing, {} no longer failing, \
                 {} not in the group, {} unstable",
                self.newly_failing.len(),
                self.no_longer_failing.len(),
                self.unknown.len(),
                self.unstable.len()
            );
        }
    }
}

/// Compare this run against the recorded list, or record this run as it.
pub(super) fn apply(report: &RunReport, path: &Path, update: bool) -> Result<bool> {
    if update {
        return record(report, path);
    }
    let baseline = Baseline::read(path)?;
    if let Some(reason) = baseline.mismatch(report) {
        return Err(format!(
            "{} does not describe this run: {reason}. A failing-case list means \
             nothing apart from the run that produced it.",
            path.display()
        )
        .into());
    }
    let comparison = Comparison::of(&baseline, report);
    comparison.write_text(path);
    Ok(comparison.agrees())
}

/// Write the run's failing ids as the new list, saying what changed.
///
/// A silent refresh is how a real regression gets enshrined as expected,
/// so the difference against the list that was there is printed first. A
/// previous file that describes some other run is not diffed -- the
/// difference would be about the run, not the shell -- but it is still
/// replaced, because re-recording after an Oils bump is exactly when this
/// is wanted.
fn record(report: &RunReport, path: &Path) -> Result<bool> {
    let previous = path.is_file().then(|| Baseline::read(path)).transpose()?;
    match previous {
        Some(previous) if previous.mismatch(report).is_none() => {
            Comparison::of(&previous, report).write_text(path)
        }
        Some(previous) => eprintln!(
            "{} described another run ({}); recording this one instead",
            path.display(),
            previous
                .mismatch(report)
                .unwrap_or_else(|| "no difference".to_owned())
        ),
        None => eprintln!("{} does not exist yet; recording it", path.display()),
    }
    let baseline = Baseline::from_run(report);
    let recorded = baseline.failing.len();
    baseline.write(path)?;
    eprintln!("baseline: wrote {recorded} failing cases");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::{CaseRecord, Options, Outcome, OutputFormat, RunReport, Totals};
    use super::*;
    use crate::process::ScratchTree;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A report holding exactly the named outcomes.
    ///
    /// Ids are given as `spec:index` because that is what a baseline
    /// holds; the record is rebuilt from the halves so the test cannot
    /// agree with the code about a format neither of them applies.
    fn report(failing: &[&str], timed_out: &[&str]) -> RunReport {
        passing_report(failing, timed_out, &[])
    }

    fn passing_report(failing: &[&str], timed_out: &[&str], passing: &[&str]) -> RunReport {
        let mut cases = Vec::new();
        for (ids, outcome) in [
            (failing, Outcome::Fail),
            (timed_out, Outcome::Timeout),
            (passing, Outcome::Pass),
        ] {
            for id in ids {
                let (spec, index) = id.rsplit_once(':').expect("id is spec:index");
                cases.push(CaseRecord {
                    spec: spec.to_owned(),
                    index: index.parse().expect("index is a number"),
                    line: 1,
                    description: String::new(),
                    outcome,
                    status: None,
                    duration_ms: 0,
                    qualifier: None,
                    differences: vec![],
                    note: None,
                });
            }
        }
        cases.push(CaseRecord {
            spec: "passing.test.sh".to_owned(),
            index: 0,
            line: 1,
            description: String::new(),
            outcome: Outcome::Pass,
            status: None,
            duration_ms: 0,
            qualifier: None,
            differences: vec![],
            note: None,
        });
        RunReport {
            schema: 1,
            survey: "oils-shell-spec",
            source_commit: "15de8fd".to_owned(),
            group: "bash-comparison".to_owned(),
            group_label: "Bash comparison survey".to_owned(),
            shell: "target/gate/bash".to_owned(),
            shell_sha256: "0".repeat(64),
            expectation_shell: "bash".to_owned(),
            containment: "sandbox".to_owned(),
            posix: false,
            timeout_ms: 5_000,
            elapsed_ms: 0,
            totals: Totals::default(),
            cases,
        }
    }

    fn options_with_baseline() -> Options {
        Options {
            root: PathBuf::from("."),
            group: "bash-comparison".to_owned(),
            shell: PathBuf::from("target/gate/bash"),
            expectation_shell: "bash".to_owned(),
            timeout: Duration::from_millis(5_000),
            format: OutputFormat::Text,
            specs: BTreeSet::new(),
            case_filter: None,
            max_cases: None,
            summary: None,
            baseline: Some(PathBuf::from("BASELINE.toml")),
            update_baseline: false,
            posix: false,
            verbose: false,
            base_path: None,
            timezone: None,
            locale_archive: None,
        }
    }

    /// The bug this whole file exists for, asked directly.
    ///
    /// `[a-z0-9-]` has no underscore, so the pipeline this replaced saw
    /// 473 of 476 failures and said so as though it were all of them.
    #[test]
    fn failing_ids_keep_the_underscore_named_specs() {
        let run = report(
            &["case_.test.sh:1", "command_.test.sh:12", "alias.test.sh:0"],
            &[],
        );
        assert_eq!(
            run.failing_ids().into_iter().collect::<Vec<_>>(),
            ["alias.test.sh:0", "case_.test.sh:1", "command_.test.sh:12"]
        );
    }

    #[test]
    fn a_matching_run_agrees_with_its_baseline() {
        let recorded = Baseline::from_run(&report(&["case_.test.sh:1"], &[]));
        let comparison = Comparison::of(&recorded, &report(&["case_.test.sh:1"], &[]));
        assert!(comparison.agrees());
        assert_eq!(comparison.recorded, 1);
        assert_eq!(comparison.observed, 1);
    }

    #[test]
    fn a_new_failure_is_named_and_refused() {
        let recorded = Baseline::from_run(&report(&["alias.test.sh:0"], &[]));
        let comparison = Comparison::of(
            &recorded,
            &report(&["alias.test.sh:0", "command_.test.sh:3"], &[]),
        );
        assert!(!comparison.agrees());
        assert_eq!(comparison.newly_failing, ["command_.test.sh:3"]);
        assert!(comparison.no_longer_failing.is_empty());
    }

    /// A case that stops failing is a difference too, whether it was
    /// fixed or merely won a race it usually loses.
    #[test]
    fn a_case_that_stops_failing_is_named() {
        let recorded = Baseline::from_run(&report(&["alias.test.sh:0", "case_.test.sh:1"], &[]));
        let comparison = Comparison::of(
            &recorded,
            &passing_report(&["alias.test.sh:0"], &[], &["case_.test.sh:1"]),
        );
        assert!(!comparison.agrees());
        assert_eq!(comparison.no_longer_failing, ["case_.test.sh:1"]);
        assert!(comparison.newly_failing.is_empty());
        assert!(comparison.unknown.is_empty());
    }

    /// A baseline entry the group no longer contains is stale, not
    /// fixed, and calling it fixed is how a corpus bump quietly deletes
    /// coverage.
    #[test]
    fn an_absent_case_is_not_a_fix() {
        let recorded = Baseline::from_run(&report(
            &["alias.test.sh:0", "case_.test.sh:1", "gone.test.sh:4"],
            &[],
        ));
        let comparison = Comparison::of(
            &recorded,
            &passing_report(&["alias.test.sh:0"], &[], &["case_.test.sh:1"]),
        );
        assert_eq!(comparison.no_longer_failing, ["case_.test.sh:1"]);
        assert_eq!(comparison.unknown, ["gone.test.sh:4"]);
        assert!(!comparison.agrees());
    }

    /// A timeout is not a failure, so the sets can agree while the run
    /// decided nothing. The verdict must not.
    #[test]
    fn a_timeout_is_never_an_expected_outcome() {
        let recorded = Baseline::from_run(&report(&["alias.test.sh:0"], &[]));
        let comparison = Comparison::of(
            &recorded,
            &report(&["alias.test.sh:0"], &["process-sub.test.sh:1"]),
        );
        assert!(comparison.newly_failing.is_empty());
        assert!(comparison.no_longer_failing.is_empty());
        assert_eq!(comparison.unstable, ["process-sub.test.sh:1"]);
        assert!(!comparison.agrees());
    }

    #[test]
    fn a_baseline_refuses_a_foreign_run() {
        let recorded = Baseline::from_run(&report(&["alias.test.sh:0"], &[]));
        let mut other = report(&["alias.test.sh:0"], &[]);
        other.group = "bash-extension".to_owned();
        let reason = recorded.mismatch(&other).expect("group differs");
        assert!(reason.contains("group"), "{reason}");
        other = report(&["alias.test.sh:0"], &[]);
        other.expectation_shell = "osh".to_owned();
        assert!(recorded.mismatch(&other).is_some());
        other = report(&["alias.test.sh:0"], &[]);
        other.source_commit = "other".to_owned();
        assert!(recorded.mismatch(&other).is_some());
        assert!(
            recorded
                .mismatch(&report(&["alias.test.sh:0"], &[]))
                .is_none()
        );
    }

    #[test]
    fn a_recorded_baseline_reads_back_as_itself() {
        let scratch = ScratchTree::new().unwrap();
        let path = scratch.path().join("BASELINE.toml");
        let run = report(&["case_.test.sh:1", "command_.test.sh:3"], &[]);
        assert!(apply(&run, &path, true).unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# The cases the bash-comparison"),
            "header missing: {text}"
        );
        assert!(text.contains("case_.test.sh:1"), "{text}");
        assert!(apply(&run, &path, false).unwrap());
        assert!(
            !apply(
                &passing_report(&["case_.test.sh:1"], &[], &["command_.test.sh:3"]),
                &path,
                false
            )
            .unwrap()
        );
    }

    /// A filtered run cannot answer a whole group's question, so it is
    /// not allowed to be asked it.
    #[test]
    fn a_filtered_run_is_not_comparable() {
        let mut options = options_with_baseline();
        options.specs.insert("alias".to_owned());
        assert!(
            options
                .check_baseline_is_answerable()
                .unwrap_err()
                .to_string()
                .contains("--spec")
        );
        options = options_with_baseline();
        options.case_filter = Some("alias".to_owned());
        assert!(options.check_baseline_is_answerable().is_err());
        options = options_with_baseline();
        options.max_cases = Some(1);
        assert!(options.check_baseline_is_answerable().is_err());
        options = options_with_baseline();
        options.update_baseline = true;
        options.max_cases = Some(1);
        assert!(options.check_baseline_is_answerable().is_err());
        assert!(
            options_with_baseline()
                .check_baseline_is_answerable()
                .is_ok()
        );
    }

    /// The checked-in list must name cases that exist, in the group and
    /// at the corpus revision it claims.
    ///
    /// Offline and cheap, so it runs with the unit tests rather than
    /// waiting for an hour-long survey. It cannot tell a stale entry
    /// from a live one -- only a run does that -- but it does catch the
    /// baseline being left behind by a corpus bump, which is the way a
    /// register goes quietly wrong between runs.
    #[test]
    fn the_checked_in_baseline_names_cases_that_exist() {
        let root = crate::survey_root();
        let baseline = Baseline::read(&root.join("BASH_COMPARISON_FAILURES.toml")).unwrap();
        let lock = crate::read_lock(&root).unwrap();
        assert_eq!(baseline.oils_commit, lock.commit);
        assert_eq!(baseline.group, "bash-comparison");
        assert_eq!(baseline.expectation_shell, "bash");
        let manifest: crate::OilsManifest =
            toml::from_str(&fs::read_to_string(root.join("MANIFEST.toml")).unwrap()).unwrap();
        let known: BTreeSet<String> = super::super::bash_case_catalog(&root, &manifest)
            .unwrap()
            .into_iter()
            .filter(|case| case.groups.iter().any(|group| group == &baseline.group))
            .map(|case| case.id)
            .collect();
        for id in &baseline.failing {
            assert!(
                known.contains(id),
                "{id} is not a case in {}",
                baseline.group
            );
        }
        assert!(baseline.failing.len() < known.len());
    }

    #[test]
    fn recording_needs_somewhere_to_write() {
        let mut options = options_with_baseline();
        options.baseline = None;
        options.update_baseline = true;
        assert!(
            options
                .check_baseline_is_answerable()
                .unwrap_err()
                .to_string()
                .contains("--baseline")
        );
    }
}
