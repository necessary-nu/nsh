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
//!
//! AND THE LIST MOVES FOR REASONS THAT ARE NOT THE SHELL'S. Two
//! `process-sub` cases are decided by a race between a `>(list)` child
//! and the shell's own exit, and the machine's load decides it in both
//! directions: measured on 2026-09-02 over 100 harness runs each,
//! `process-sub.test.sh:2` is in this list as an expected failure and
//! passed 41 times at load 87, while `:1` is not in it and failed about 9
//! times in 100. So a loaded run reported a difference either way and a
//! quiet one did not, and the file said nothing about it.
//!
//! `crate::control` is the answer, and it is the one `gate-bash` already
//! had: when the comparison turns on a case, the pinned Bash is asked
//! whether it still reproduces its own recorded result on the disputed
//! spec files. A case it cannot is undecided this run -- named, with the
//! count behind it, and left out of the verdict. It is asked of the
//! reference alone, so a case this shell loses that the reference wins
//! every time is still a difference at any load.
//!
//! THE GENERATOR NEEDED IT MORE THAN THE COMPARISON DID. A refresh takes
//! the run's failing set as the new list, so a lucky run silently
//! *deletes* a known-flaky entry -- which is what happened to
//! `process-sub.test.sh:2` on 2026-09-01, found only because someone
//! remembered it should be there. An undecided case now keeps the answer
//! already recorded, in both directions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::{Result, RunReport};
use crate::control::{Control, contended_cases};
use crate::provenance::Provenance;

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
#     nsh-survey run-oils --expect-shell {expectation_shell} \\
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
# A case the pinned Bash could not reproduce its own recorded result on is
# undecided: the comparison names it and does not count it, and a refresh
# keeps whatever this file already said about it rather than taking the
# lucky run's answer. `process-sub.test.sh:2` was deleted from here that way
# on 2026-09-01 and had to be measured again by hand.
#
# The pins are what make the list mean something. A run whose group,
# expectation namespace, POSIX mode, per-case timeout or Oils commit differs
# from these is refused rather than compared, and a filtered run (`--spec`,
# `--case`, `--max-cases`) is refused outright because every unselected case
# would read as fixed. The shell's hash is not pinned; it changes with every
# build.
#
# `nsh_commit`, `shell_sha256` and `uncommitted` are the opposite of a pin:
# they say what this list came from and are never compared against anything.
# A refresh refuses a checkout carrying uncommitted work, because the shell
# it measured was built from that work and no commit explains the result;
# `--update-baseline-from-dirty-tree` records the list anyway and names every
# such path in `uncommitted`. `ee98cec` had no way to do that and attributed
# two entries it removed to a commit that did not remove them.

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
    /// The bytes this list was measured from, and the checkout they were
    /// built in. Never pinned -- see [`Baseline::mismatch`] -- because
    /// both change with every build and pinning either would refuse the
    /// only runs anyone wants to make. They are here so the file can say
    /// what it was, which is the one thing `ee98cec` could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shell_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nsh_commit: Option<String>,
    /// Everything in that checkout that no commit accounts for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    uncommitted: Vec<String>,
    failing: BTreeSet<String>,
}

impl Baseline {
    /// The pins from this run, with the failing set given rather than
    /// taken.
    ///
    /// `record` adjusts the set for the cases the run could not decide,
    /// and a constructor that read `report.failing_ids()` itself would
    /// silently undo that.
    fn from_run(report: &RunReport, failing: BTreeSet<String>, taken_in: &Provenance) -> Self {
        Self {
            schema: SCHEMA,
            group: report.group.clone(),
            expectation_shell: report.expectation_shell.clone(),
            posix: report.posix,
            timeout_ms: report.timeout_ms,
            oils_commit: report.source_commit.clone(),
            shell_sha256: Some(report.shell_sha256.clone()),
            nsh_commit: Some(taken_in.commit.clone()),
            uncommitted: taken_in.uncommitted.clone(),
            failing,
        }
    }

    /// What this list can be attributed to, in one line.
    ///
    /// Printed by every comparison, because a difference read against a
    /// list of unknown provenance is a difference nobody can act on. A
    /// file recorded before this was tracked says so rather than
    /// pretending to a clean tree.
    fn taken_in(&self) -> String {
        match (&self.nsh_commit, self.uncommitted.len()) {
            (None, _) => "recorded before the runner tracked which checkout it measured; \
                 the shell behind it cannot be attributed to a commit"
                .to_owned(),
            (Some(commit), 0) => {
                format!("recorded at {commit}, with nothing uncommitted in the checkout")
            }
            (Some(commit), count) => format!(
                "recorded at {commit} over {count} uncommitted path(s), whose effects are \
                 in this list and in no commit: {}",
                self.uncommitted.join(", ")
            ),
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
    /// The cases the reference could not reproduce itself on this run.
    ///
    /// They start in one of the three lists above and are moved here by
    /// `set_aside`, so a case is never both a difference and undecided.
    undecided: Vec<String>,
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
            undecided: Vec::new(),
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

    /// The cases worth asking the reference about.
    ///
    /// Every one of these is a live outcome: this run's shell did
    /// something the recorded list does not expect. The ids the list
    /// names and the group no longer contains are left out on purpose --
    /// no rerun of anything can change what a checked-in file says, and
    /// the control's budget would be spent on a question no run answers.
    fn disputed(&self) -> BTreeSet<String> {
        self.newly_failing
            .iter()
            .chain(&self.no_longer_failing)
            .chain(&self.unstable)
            .cloned()
            .collect()
    }

    /// Move the cases the reference could not reproduce out of the
    /// verdict and into `undecided`.
    ///
    /// A case is never both: it is a difference the run measured, or it
    /// is a case this machine could not measure at all.
    fn set_aside(&mut self, contended: &BTreeSet<String>) {
        for list in [
            &mut self.newly_failing,
            &mut self.no_longer_failing,
            &mut self.unstable,
        ] {
            list.retain(|id| {
                let keep = !contended.contains(id);
                if !keep {
                    self.undecided.push(id.clone());
                }
                keep
            });
        }
        self.undecided.sort();
        self.undecided.dedup();
    }

    fn undecided(&self) -> &[String] {
        &self.undecided
    }

    /// Report to stderr, deliberately.
    ///
    /// The verdict is about the run rather than part of its output, and
    /// stdout already carries a whole document under `--format json` and
    /// a machine-readable list under `--format ids`. Writing the verdict
    /// there would corrupt both, and a check nobody can pipe is a check
    /// people stop running.
    fn write_text(&self, path: &Path, control: &Control, taken_in: &str) {
        eprintln!("failing-case baseline: {}", path.display());
        eprintln!("{taken_in}");
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
        if !self.undecided.is_empty() {
            eprintln!("{}", Control::headline(self.undecided.len()));
            for id in &self.undecided {
                eprintln!(
                    "  ~ {id} -- the pinned Bash lost it in {} of {} control runs",
                    control.lost(id),
                    control.runs()
                );
            }
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

/// Ask the reference about the cases this run's verdict turns on.
///
/// The same control `gate-bash` uses, on the same budget, against the
/// same recording of what the pinned Bash does: a case the reference
/// cannot reproduce itself on cannot say anything about this shell
/// either. It is paid for only when the comparison found something, so
/// an agreeing run costs exactly what it always did.
///
/// A run that expects some other shell's output is left uncontrolled and
/// says so. `BASH_REFERENCE_CASES.json` records what the pinned Bash
/// does, so it can answer for `--expect-shell bash` and for nothing else.
fn control_for(
    report: &RunReport,
    manifest: &crate::OilsManifest,
    root: &Path,
    comparison: &Comparison,
) -> Result<Control> {
    let disputed = comparison.disputed();
    if disputed.is_empty() {
        return Ok(Control::default());
    }
    if report.expectation_shell != "bash" {
        eprintln!(
            "baseline: no control run -- the reference calibration answers for \
             --expect-shell bash and this run expects {}",
            report.expectation_shell
        );
        return Ok(Control::default());
    }
    let (eligible, _) = crate::bash_reference::calibration(root)?;
    contended_cases(root, manifest, &report.group, &eligible, &disputed)
}

/// Compare this run against the recorded list, or record this run as it.
pub(super) fn apply(
    report: &RunReport,
    manifest: &crate::OilsManifest,
    root: &Path,
    path: &Path,
    taken_in: Option<Provenance>,
    overwrite: bool,
) -> Result<bool> {
    if let Some(taken_in) = taken_in {
        return record(report, manifest, root, path, &taken_in, overwrite);
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
    let mut comparison = Comparison::of(&baseline, report);
    let control = control_for(report, manifest, root, &comparison)?;
    comparison.set_aside(&control.contended());
    comparison.write_text(path, &control, &baseline.taken_in());
    Ok(comparison.agrees())
}

/// Take the recorded answer for every case this run could not decide.
///
/// Both directions matter and only one of them has bitten so far. A case
/// the list expects to fail and this run passed stays in the list, which
/// is the `process-sub.test.sh:2` repair; a case the list expects to pass
/// and this run failed stays out of it, so a race cannot enshrine itself
/// as an expected failure either.
fn keep_recorded_answer(
    failing: &mut BTreeSet<String>,
    recorded: &BTreeSet<String>,
    undecided: &[String],
) -> usize {
    for id in undecided {
        let was_failing = recorded.contains(id);
        if was_failing {
            failing.insert(id.clone());
        } else {
            failing.remove(id);
        }
        eprintln!(
            "  = {id} keeps its recorded answer ({}); this run could not decide it",
            if was_failing { "failing" } else { "passing" }
        );
    }
    undecided.len()
}

/// Write the run's failing ids as the new list, saying what changed.
///
/// A silent refresh is how a real regression gets enshrined as expected,
/// so the difference against the list that was there is printed first. A
/// previous file that describes some other run is not diffed -- the
/// difference would be about the run, not the shell -- but it is still
/// replaced, because re-recording after an Oils bump is exactly when this
/// is wanted.
///
/// A REFRESH KEEPS WHAT THIS RUN COULD NOT DECIDE. On 2026-09-01 a lucky
/// run of `--update-baseline` dropped `process-sub.test.sh:2`, a case
/// both shells lose to a race about four runs in five on a quiet machine,
/// and the entry had to be measured again and put back by hand. A
/// generated file that quietly loses an entry on a good day is the defect
/// the whole baseline exists to stop, wearing the generator's hat. So an
/// undecided case takes the answer already recorded rather than this
/// run's, in both directions -- a case the run newly failed is not
/// enshrined as expected either -- and every one of them is named.
///
/// AND IT REFUSES TO WRITE OVER A CHANGE IT DID NOT MAKE. The last thing
/// before the write is `guard_generated`, asked again rather than only
/// before the run: the group run is minutes long and is exactly the
/// window in which another session re-records this file. On 2026-09-02 a
/// refresh did write over one, and the three ids it discarded were
/// reconstructed by hand from the comparison's own output.
///
/// AND IT RECORDS WHAT IT MEASURED. `taken_in` has already refused a
/// checkout carrying uncommitted work unless the caller spelled out
/// `--update-baseline-from-dirty-tree`; what reaches here is written into
/// the file, so a list taken over half-finished work says so forever
/// instead of being attributed to whatever commit happened to land next.
fn record(
    report: &RunReport,
    manifest: &crate::OilsManifest,
    root: &Path,
    path: &Path,
    taken_in: &Provenance,
    overwrite: bool,
) -> Result<bool> {
    let previous = path.is_file().then(|| Baseline::read(path)).transpose()?;
    let mut failing = report.failing_ids();
    let mut kept = 0_usize;
    match previous {
        Some(previous) if previous.mismatch(report).is_none() => {
            let mut comparison = Comparison::of(&previous, report);
            let control = control_for(report, manifest, root, &comparison)?;
            comparison.set_aside(&control.contended());
            comparison.write_text(path, &control, &previous.taken_in());
            kept = keep_recorded_answer(&mut failing, &previous.failing, comparison.undecided());
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
    let baseline = Baseline::from_run(report, failing, taken_in);
    let recorded = baseline.failing.len();
    let attribution = baseline.taken_in();
    crate::provenance::guard_generated(path, overwrite)?;
    baseline.write(path)?;
    eprintln!(
        "baseline: wrote {recorded} failing cases, {kept} of them unchanged because \
               this run could not decide them"
    );
    eprintln!("baseline: {attribution}");
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

    /// A baseline recorded from a run, the way `record` records one.
    fn baseline_of(run: &RunReport) -> Baseline {
        Baseline::from_run(run, run.failing_ids(), &taken_in(&[]))
    }

    /// A reading of a checkout, without asking git for one.
    fn taken_in(uncommitted: &[&str]) -> Provenance {
        Provenance {
            commit: "b028f47".to_owned(),
            uncommitted: uncommitted.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    fn options_with_baseline() -> Options {
        Options {
            root: PathBuf::from("."),
            group: "bash-comparison".to_owned(),
            shell: PathBuf::from("target/gate/bash"),
            reported_shell: "target/gate/bash".to_owned(),
            expectation_shell: "bash".to_owned(),
            timeout: Duration::from_millis(5_000),
            format: OutputFormat::Text,
            specs: BTreeSet::new(),
            case_filter: None,
            max_cases: None,
            summary: None,
            baseline: Some(PathBuf::from("BASELINE.toml")),
            refresh: super::super::Refresh::No,
            overwrite: false,
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
        let recorded = baseline_of(&report(&["case_.test.sh:1"], &[]));
        let comparison = Comparison::of(&recorded, &report(&["case_.test.sh:1"], &[]));
        assert!(comparison.agrees());
        assert_eq!(comparison.recorded, 1);
        assert_eq!(comparison.observed, 1);
    }

    #[test]
    fn a_new_failure_is_named_and_refused() {
        let recorded = baseline_of(&report(&["alias.test.sh:0"], &[]));
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
        let recorded = baseline_of(&report(&["alias.test.sh:0", "case_.test.sh:1"], &[]));
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
        let recorded = baseline_of(&report(
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
        let recorded = baseline_of(&report(&["alias.test.sh:0"], &[]));
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
        let recorded = baseline_of(&report(&["alias.test.sh:0"], &[]));
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

    /// The file the generator writes is the file the comparison reads.
    ///
    /// Driven through `Baseline` rather than through `apply`, because
    /// `apply` asks the pinned Bash about anything it cannot decide and
    /// this is a question about a file. What the control does with a
    /// disagreement is `the_undecided_leave_the_verdict`'s and
    /// `an_undecided_case_keeps_its_recorded_answer`'s.
    #[test]
    fn a_recorded_baseline_reads_back_as_itself() {
        let scratch = ScratchTree::new().unwrap();
        let path = scratch.path().join("BASELINE.toml");
        let run = report(&["case_.test.sh:1", "command_.test.sh:3"], &[]);
        baseline_of(&run).write(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# The cases the bash-comparison"),
            "header missing: {text}"
        );
        assert!(text.contains("case_.test.sh:1"), "{text}");
        let read_back = Baseline::read(&path).unwrap();
        assert!(Comparison::of(&read_back, &run).agrees());
        assert!(
            !Comparison::of(
                &read_back,
                &passing_report(&["case_.test.sh:1"], &[], &["command_.test.sh:3"])
            )
            .agrees()
        );
    }

    /// A refresh taken over uncommitted work says so in the file it
    /// writes.
    ///
    /// `ee98cec` removed `assign.test.sh:19` and `assign.test.sh:45`
    /// from this list and attributed both to a commit that did not
    /// remove them: the shell it measured had another session's
    /// uncommitted files built into it, and nothing in the file it wrote
    /// could say so. Now the file names them, and the comparison prints
    /// the sentence on every run.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn a_refresh_records_the_work_no_commit_explains() {
        let scratch = ScratchTree::new().unwrap();
        let path = scratch.path().join("BASELINE.toml");
        let run = report(&["case_.test.sh:1"], &[]);
        let over = taken_in(&["crates/nsh/src/variables.rs"]);
        Baseline::from_run(&run, run.failing_ids(), &over)
            .write(&path)
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("crates/nsh/src/variables.rs"), "{text}");

        let read_back = Baseline::read(&path).unwrap();
        let attribution = read_back.taken_in();
        for named in ["b028f47", "crates/nsh/src/variables.rs"] {
            assert!(attribution.contains(named), "{attribution}");
        }
        assert!(
            Comparison::of(&read_back, &run).agrees(),
            "the provenance became a pin and refused its own run",
        );
        assert!(
            Baseline::from_run(&run, run.failing_ids(), &taken_in(&[]))
                .taken_in()
                .contains("nothing uncommitted"),
            "a clean checkout did not say so",
        );

        /* The list checked in before this existed carries no provenance
         * at all, and a comparison against it must say that rather than
         * read as a clean tree. */
        let older: Baseline = toml::from_str(
            "schema = 1\ngroup = \"g\"\nexpectation_shell = \"bash\"\nposix = false\n\
             timeout_ms = 5000\noils_commit = \"15de8fd\"\nfailing = []\n",
        )
        .unwrap();
        assert!(
            older.taken_in().contains("cannot be attributed"),
            "{}",
            older.taken_in()
        );
    }

    /// The control's answer takes a case out of the verdict, and only
    /// the cases it names.
    ///
    /// A case the reference reproduced is still a difference however
    /// loaded the machine was: that is the property that keeps this a
    /// control and not a retry.
    #[test]
    fn the_undecided_leave_the_verdict() {
        let recorded = baseline_of(&report(&["alias.test.sh:0", "case_.test.sh:1"], &[]));
        let run = passing_report(
            &["alias.test.sh:0", "command_.test.sh:3"],
            &["process-sub.test.sh:1"],
            &["case_.test.sh:1"],
        );
        let mut comparison = Comparison::of(&recorded, &run);
        assert_eq!(
            comparison.disputed(),
            [
                "case_.test.sh:1".to_owned(),
                "command_.test.sh:3".to_owned(),
                "process-sub.test.sh:1".to_owned(),
            ]
            .into_iter()
            .collect::<BTreeSet<_>>(),
            "the disputed set is the live outcomes and nothing else",
        );
        assert!(!comparison.agrees());

        comparison.set_aside(
            &[
                "case_.test.sh:1".to_owned(),
                "process-sub.test.sh:1".to_owned(),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(
            comparison.undecided(),
            [
                "case_.test.sh:1".to_owned(),
                "process-sub.test.sh:1".to_owned()
            ]
        );
        assert_eq!(comparison.newly_failing, ["command_.test.sh:3"]);
        assert!(comparison.no_longer_failing.is_empty());
        assert!(comparison.unstable.is_empty());
        assert!(
            !comparison.agrees(),
            "a case the reference reproduced is still a difference",
        );

        comparison.set_aside(&["command_.test.sh:3".to_owned()].into_iter().collect());
        assert!(comparison.agrees(), "nothing decided is left to disagree");
    }

    /// The refresh this node exists for.
    ///
    /// `--update-baseline` dropped `process-sub.test.sh:2` on a lucky run
    /// on 2026-09-01 and it had to be measured again and put back by
    /// hand. An undecided case now takes the recorded answer in both
    /// directions.
    #[test]
    fn an_undecided_case_keeps_its_recorded_answer() {
        let recorded: BTreeSet<String> = ["process-sub.test.sh:2".to_owned()].into_iter().collect();
        let mut failing: BTreeSet<String> = BTreeSet::new();
        assert_eq!(
            keep_recorded_answer(
                &mut failing,
                &recorded,
                &["process-sub.test.sh:2".to_owned()]
            ),
            1
        );
        assert!(
            failing.contains("process-sub.test.sh:2"),
            "a lucky run dropped the entry again",
        );

        let mut failing: BTreeSet<String> =
            ["process-sub.test.sh:1".to_owned()].into_iter().collect();
        keep_recorded_answer(
            &mut failing,
            &recorded,
            &["process-sub.test.sh:1".to_owned()],
        );
        assert!(
            failing.is_empty(),
            "an unlucky run enshrined a race as an expected failure",
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
        options.refresh = super::super::Refresh::FromCommitted;
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
        options.refresh = super::super::Refresh::FromCommitted;
        assert!(
            options
                .check_baseline_is_answerable()
                .unwrap_err()
                .to_string()
                .contains("--baseline")
        );
    }
}
