//! The control that asks whether a case can still measure a shell.
//!
//! Two checks in this crate compare a shell against a recorded verdict:
//! `gate-bash` against `BASH_DISPOSITIONS.toml`, and `run-oils
//! --baseline` against a failing-case list. Both are decided by cases
//! that a loaded machine can flip in either direction, and neither can
//! tell "this shell changed" from "this machine was busy" by looking at
//! the shell alone.
//!
//! So when a verdict turns on a case, the *reference* is asked whether it
//! still reproduces its own recorded result on this machine, on the spec
//! files in dispute, several times over. A case it cannot reproduce is
//! undecided this run: reported with the count behind it, and left out of
//! the verdict.
//!
//! IT IS NOT A RETRY, and the difference is the whole design. A retry
//! asks the shell again and takes the better answer, which hides exactly
//! the regressions that only appear under load. This asks a different
//! question, of a different shell, and a case this shell loses that the
//! reference wins every time is still a difference -- at any load, however
//! many times it is asked.
//!
//! ONE IMPLEMENTATION, TWO CALLERS. The gate grew this on 2026-09-02 and
//! `run-oils --baseline` had none, which left the repository's documented
//! one-command comparison load-dependent in both directions on the same
//! `process-sub` cases the gate had just learned to survive. A second
//! copy would have been a second set of constants to measure.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::oils_runner::{GateOutcome, run_gate_specs};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// How many times the control asks the reference before it believes it.
///
/// Measured rather than reasoned, on 2026-09-02 at load 84 to 87: the
/// pinned Bash lost `process-sub.test.sh:1` in 45 of 300 runs. Cutting
/// that record into consecutive blocks says what each size of control
/// would have concluded -- three runs saw the race in 37 of 100 blocks,
/// fifteen saw it in 19 of 20. Fifteen also costs less than three did,
/// because the control now asks only about the files in dispute rather
/// than re-running the group.
///
/// WHAT IT BOUGHT, AND IT IS LESS THAN THAT ARITHMETIC PREDICTS. Over 48
/// gate runs at load 78 to 84 the gate failed 3 times, always on
/// `process-sub.test.sh:1`, against the 1 run in 9 recorded on `750758b`.
/// The control fired 5 times in those 48 and excused 2, with the
/// reference losing the case 2 of 15 and 4 of 15 when it did and 0 of 15
/// each of the three times it did not. A per-run rate of about 10% --
/// measured separately at the same load, 22 losses in 200 for the
/// reference and 24 for this shell -- predicts 15 runs seeing the race
/// 79% of the time, and 2 of 5 is well under that.
///
/// So a bigger constant is not obviously the remaining fix: three misses
/// of 0 of 15 at a 10% rate is a 1-in-1000 coincidence, which points at
/// the control's runs not being independent of the moment the gate's own
/// run lost the case -- the group run takes 40 seconds and spans the
/// machine's slow phases, while fifteen nine-case runs finish inside one.
/// Raising the count would also make the control likelier to excuse a
/// case where the reference is barely flaky and this shell has a real
/// regression, which is the failure this whole control must not become.
/// Left at fifteen, and the count behind every undecided case is now
/// printed so the next person can see this rather than infer it.
///
/// THAT GUESS WAS TESTED ON 2026-09-02 AND IS NOT SUPPORTED. If a control
/// run were a quieter place than the group run it is checking, the
/// reference would lose the race less often in one. Measured interleaved
/// round by round at load 89 to 112, whole-group reference runs against
/// fifteen one-file runs each: the reference lost `process-sub.test.sh:1`
/// in 2 of 9 group runs and 26 of 135 one-file runs -- 22% against 19%,
/// which nine group samples cannot separate from equal. So the shape of
/// the control's run is not visibly the reason, and the three misses stay
/// unexplained rather than explained badly.
///
/// The mechanism is real for at least one case, in the other shell.
/// `sh-options.test.sh:23` fails 6 of 6 runs on its own at load 94, 4 of
/// 6 when its whole spec file runs, and 6 of 10 in the whole group: same
/// case, same load, stable alone and unstable behind other cases. That is
/// `stop-the-noclobber-append-case-flaking`, and it is why the question
/// above is worth asking again with more group samples than nine.
const CONTROL_RUNS: usize = 15;

/// The dialect is selected by `argv[0]`, so a shell under any other name
/// measures the profile with the profile turned off.
///
/// The control runs the pinned Bash under whatever name it is installed
/// with, so this guards the control as well as the gate: a reference
/// answering as `bash-reference` would be answering a different
/// question.
pub(crate) fn require_bash_basename(shell: &Path) -> Result<()> {
    if shell.file_name() == Some(OsStr::new("bash")) {
        return Ok(());
    }
    Err(format!(
        "the shell must be named exactly `bash` -- {} would run with the dialect off and report bogus numbers",
        shell.display()
    )
    .into())
}

/// Where the pinned Bash is, for the control run.
///
/// The gate already depends on this build: `BASH_REFERENCE_CASES.json` is
/// what it produced. Naming the binary too is the difference between
/// comparing this shell against a recording made on a quiet machine and
/// comparing it against the reference on the machine the gate is running
/// on.
///
/// It is found from this crate rather than from the survey root the
/// caller named, and that is a 2026-09-02 fix rather than a preference.
/// The reference is a build artefact under `target/`, not part of a
/// survey root, so `root.join("../../../target/...")` only resolved when
/// the root was the default one -- and `tests/harness/bash-gate-selftest.sh`
/// runs every one of its eight mutations against a *copy*. The control
/// could therefore never run there: "stale entry on a passing case" was
/// refused with "the gate needs the pinned Bash ... is not there" rather
/// than for its own reason, on this change and on `750758b` alike.
pub(crate) fn pinned_reference() -> Result<PathBuf> {
    let path = env::var_os("NSH_FUZZ_BASH").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bash-reference/bash"),
        PathBuf::from,
    );
    let path = fs::canonicalize(&path).map_err(|error| {
        format!(
            "the gate needs the pinned Bash for its control run and {} is not there ({error}); \
             build it with `nsh-survey build-bash-reference` or name it with NSH_FUZZ_BASH",
            path.display()
        )
    })?;
    Ok(path)
}

/// What the control saw, kept as a count rather than a verdict.
///
/// The report prints it beside every undecided case, because "the
/// reference lost this 6 times in 15" and "the reference lost it once in
/// 15" are different claims and only one of them says the case cannot
/// measure a shell. A control that printed neither would be asking the
/// reader to take its arithmetic on trust, which is the shape
/// `[spec:nsh:req:oracle.cannot-measure-is-a-failure]` objects to.
#[derive(Default)]
pub(crate) struct Control {
    runs: usize,
    losses: BTreeMap<String, usize>,
}

impl Control {
    /// The cases the reference could not reproduce itself on.
    pub(crate) fn contended(&self) -> BTreeSet<String> {
        self.losses.keys().cloned().collect()
    }

    /// How many of the control's runs the reference lost this case in.
    pub(crate) fn lost(&self, id: &str) -> usize {
        self.losses.get(id).copied().unwrap_or_default()
    }

    /// How many times the control asked.
    pub(crate) fn runs(&self) -> usize {
        self.runs
    }

    /// The one sentence both callers print above the per-case counts.
    pub(crate) fn headline(count: usize) -> String {
        format!(
            "undecided this run: {count} -- the pinned Bash did not reproduce its own \
             recorded result on this machine, so these measure the machine rather \
             than the shell"
        )
    }
}

/// How many control runs the disputed files are worth.
///
/// The budget is one group run. A control that costs more than the gate
/// it is checking would be paid on every red run, including the ones
/// where the failure is plainly not a race.
fn control_runs(manifest: &crate::OilsManifest, group: &str, specs: &BTreeSet<String>) -> usize {
    let mut group_cases = 0_usize;
    let mut selected_cases = 0_usize;
    for entry in &manifest.specs {
        if !entry.groups.iter().any(|candidate| candidate == group) {
            continue;
        }
        group_cases += entry.cases;
        let name = Path::new(&entry.path)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or(entry.path.as_str());
        if specs.contains(name) {
            selected_cases += entry.cases;
        }
    }
    if selected_cases == 0 {
        return 0;
    }
    (group_cases / selected_cases).clamp(1, CONTROL_RUNS)
}

/// The cases this run cannot decide, because the reference did not
/// reproduce its own recorded result on this machine.
///
/// Not a retry. A retry would hide a real regression that only shows up
/// under contention; this asks a different question -- whether the case
/// is still measuring the shell at all -- and answers it with the one
/// thing that can tell the difference, which is the reference. The
/// question is asked of the reference alone for that reason: a case this
/// shell loses under load and the reference wins every time is still
/// reported, however many times it is asked.
///
/// `process-sub.test.sh:1` is the case that made this necessary:
/// `seq 3 > >(tac)` writes from a process neither shell waits for, so the
/// sandbox tears the process substitution down when the shell exits and
/// the child may not have written yet.
///
/// Measured 2026-09-01 at load 65, this shell lost that race in 15 runs
/// of 20 and the pinned Bash lost it in 4 of 20. Neither number is a
/// property of either shell.
///
/// CORRECTION 2026-09-02: those two numbers do not reproduce, on this
/// tree or on `aa25c1f` before `750758b` rewrote the waiting. They were
/// taken one shell after the other, and a rate measured under load says
/// nothing unless both shells meet the same load, so every figure below
/// is interleaved run for run.
///
/// The headline, with all three shells built for the measurement and
/// kept where nothing else writes -- 100 harness runs each at load 87:
/// the pinned Bash lost the case 11 times, this shell 9, and the
/// pre-`750758b` build 10. Where they were kept is part of the
/// measurement: `target/bash-mode/bash` was rebuilt by another agent
/// mid-session, so three earlier runs cannot say which binary they
/// scored. They agree in direction anyway -- at load 71, Bash 18 and this
/// shell 4 in 100; at load 81, Bash 29, this shell 17 and pre-`750758b`
/// 12 in 100; under a fork-heavy load at 88, Bash 11 and this shell 6 in
/// 60.
///
/// So this shell does not lose the race more often than the reference,
/// on either tree, under a spinning load or a forking one. The surviving
/// gate failure was this control missing the reference's own loss.
pub(crate) fn contended_cases(
    root: &Path,
    manifest: &crate::OilsManifest,
    group: &str,
    eligible: &BTreeSet<String>,
    disputed: &BTreeSet<String>,
) -> Result<Control> {
    if disputed.is_empty() {
        return Ok(Control::default());
    }
    let reference = pinned_reference()?;
    require_bash_basename(&reference)?;
    /* The control asks only about the files the verdict turned on. It
     * used to re-run the whole group, which put a hard ceiling of three
     * samples on a question that needs a dozen -- the cost of a sample was
     * 873 cases and the answer wanted was about one. Narrowing it to the
     * disputed files buys the samples for less than the old control
     * spent. */
    let specs: BTreeSet<String> = disputed
        .iter()
        .filter_map(|id| id.rsplit_once(':').map(|(spec, _)| spec.to_owned()))
        .collect();
    /* Whatever the shape of the failure, the control costs at most one
     * more run of the group. A race shows up in one file and gets every
     * sample; a breakage across the whole group gets one, which is all a
     * breakage across the whole group needs. */
    let runs = control_runs(manifest, group, &specs);
    println!(
        "control: asking the pinned Bash about {} disputed case(s) in {} file(s), {runs} run(s)",
        disputed.len(),
        specs.len(),
    );
    let mut control = Control {
        runs,
        losses: BTreeMap::new(),
    };
    for _ in 0..runs {
        let live = run_gate_specs(root, manifest, &reference, group, &specs)?;
        for case in live {
            let passed = case.outcome == GateOutcome::Pass;
            if disputed.contains(&case.id) && passed != eligible.contains(&case.id) {
                *control.losses.entry(case.id).or_default() += 1;
            }
        }
    }
    Ok(control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> crate::OilsManifest {
        let root = crate::survey_root();
        toml::from_str(&fs::read_to_string(root.join("MANIFEST.toml")).unwrap()).unwrap()
    }

    /// The control's samples go where the question is.
    ///
    /// One disputed file out of a group of 873 cases is worth every run
    /// the constant allows; a dispute spanning the group is worth one,
    /// because the budget is a single extra group run and a failure that
    /// broad is not a race. This is the arithmetic the 2026-09-02
    /// measurement corrected: three runs could not see a race the
    /// reference loses once in five.
    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn the_control_spends_its_budget_on_disputed_files() {
        let manifest = manifest();
        let group = "bash-extension";

        let one: BTreeSet<String> = ["process-sub.test.sh".to_owned()].into_iter().collect();
        assert_eq!(
            control_runs(&manifest, group, &one),
            CONTROL_RUNS,
            "one nine-case file did not earn every control run",
        );

        let whole: BTreeSet<String> = manifest
            .specs
            .iter()
            .filter(|entry| entry.groups.iter().any(|candidate| candidate == group))
            .map(|entry| {
                Path::new(&entry.path)
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            control_runs(&manifest, group, &whole),
            1,
            "a dispute covering the group cost more than one extra group run",
        );

        assert_eq!(
            control_runs(&manifest, group, &BTreeSet::new()),
            0,
            "nothing disputed still bought a control run",
        );
    }

    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn another_shell_name_is_refused() {
        assert!(require_bash_basename(Path::new("/tmp/bash")).is_ok());
        assert!(require_bash_basename(Path::new("/tmp/bash-pre")).is_err());
        assert!(require_bash_basename(Path::new("/tmp/nsh")).is_err());
    }
}
