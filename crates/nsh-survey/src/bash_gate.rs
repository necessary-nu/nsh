//! The closure gate for the Bash compatibility profile.
//!
//! `[spec:nsh:req:compat.bash.survey-closure]` asks for zero *unexpected*
//! failures, which is a claim about a register rather than about a score.
//! Three files decide what "expected" means, and none of them can be
//! silently widened:
//!
//!   * `MANIFEST.toml` fixes which cases the group contains.
//!   * `BASH_REFERENCE_CASES.json` fixes which of them the pinned Bash 5.3
//!     build itself passes -- the eligible manifest -- and carries a
//!     disposition for every case it does not.
//!   * `BASH_DISPOSITIONS.toml` carries one entry for every eligible case
//!     *this* shell does not pass, with the category and the reason.
//!
//! The gate is symmetric on purpose. An unregistered case that stops
//! passing fails it, and a registered case that starts passing fails it
//! too: a stale excuse is how a real regression eventually gets waved
//! through.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::oils_runner::{GateCase, GateOutcome, run_gate_group, run_gate_specs};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const REGISTER_FILE: &str = "BASH_DISPOSITIONS.toml";
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
const CONTROL_RUNS: usize = 15;
const GROUP: &str = "bash-extension";

/// The categories a non-passing eligible case may carry.
///
/// `NotImplemented` is listed first because it is the one that must never
/// be able to hide among the others: if closure lets "we have not built
/// it" read as "we decided against it", the register is worse than
/// nothing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Category {
    NotImplemented,
    Defect,
    SanctionedDivergence,
    HarnessArtifact,
    OutOfContract,
}

impl Category {
    const fn label(self) -> &'static str {
        match self {
            Self::NotImplemented => "not-implemented",
            Self::Defect => "defect",
            Self::SanctionedDivergence => "sanctioned-divergence",
            Self::HarnessArtifact => "harness-artifact",
            Self::OutOfContract => "out-of-contract",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    schema: u32,
    group: String,
    oils_commit: String,
    #[serde(default)]
    scope: Vec<ScopeEntry>,
    #[serde(default, rename = "case")]
    cases: Vec<CaseEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeEntry {
    spec: String,
    disposition: Category,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseEntry {
    id: String,
    disposition: Category,
    reason: String,
}

pub(crate) fn command(mut args: env::ArgsOs, default_root: PathBuf) -> Result<bool> {
    let mut shell = None;
    let mut root = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--shell") => {
                shell = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--shell requires a path")?,
                );
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown gate-bash option {value:?}; {}", usage()).into());
            }
            _ if root.is_none() => root = Some(PathBuf::from(argument)),
            _ => return Err(format!("unexpected argument; {}", usage()).into()),
        }
    }
    let shell = shell.ok_or_else(|| format!("gate-bash requires --shell PATH; {}", usage()))?;
    gate(&root.unwrap_or(default_root), &shell)
}

fn usage() -> &'static str {
    "usage: nsh-survey gate-bash --shell PATH [ROOT]"
}

/// The dialect is selected by `argv[0]`, so a shell under any other name
/// measures the profile with the profile turned off.
fn require_bash_basename(shell: &Path) -> Result<()> {
    if shell.file_name() == Some(OsStr::new("bash")) {
        return Ok(());
    }
    Err(format!(
        "the gate shell must be named exactly `bash` -- {} would run with the dialect off and report bogus numbers",
        shell.display()
    )
    .into())
}

fn read_register(root: &Path) -> Result<Register> {
    let path = root.join(REGISTER_FILE);
    let register: Register = toml::from_str(&fs::read_to_string(&path)?)?;
    if register.schema != 1 {
        return Err(format!("{} has unsupported schema", path.display()).into());
    }
    if register.group != GROUP {
        return Err(format!("{} names group {}", path.display(), register.group).into());
    }
    for entry in &register.cases {
        if entry.reason.trim().is_empty() {
            return Err(format!("{} has no reason", entry.id).into());
        }
    }
    for entry in &register.scope {
        if entry.reason.trim().is_empty() {
            return Err(format!("{} has no reason", entry.spec).into());
        }
        /* A whole file leaves the contract only by the scope decision. Any
         * other category would be a claim about one case, made about many. */
        if entry.disposition != Category::OutOfContract {
            return Err(format!(
                "scope entry {} is {}; a whole file can only be out-of-contract",
                entry.spec,
                entry.disposition.label()
            )
            .into());
        }
    }
    Ok(register)
}

struct Findings {
    violations: Vec<String>,
    /// The cases whose *live outcome* raised a violation.
    ///
    /// Only these can be contended: the rest of the verdict is
    /// bookkeeping over three checked-in files, which running anything
    /// again cannot change. This is what the control re-asks about, and
    /// keeping it separate from `violations` is what stops the control
    /// re-running the whole group to settle one case.
    disputed: BTreeSet<String>,
    passing: usize,
    counts: BTreeMap<Category, usize>,
    out_of_contract: usize,
    reference_excluded: BTreeMap<String, usize>,
    contended: BTreeSet<String>,
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
fn pinned_reference() -> Result<PathBuf> {
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
fn contended_cases(
    root: &Path,
    manifest: &crate::OilsManifest,
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
    let runs = control_runs(manifest, &specs);
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
        let live = run_gate_specs(root, manifest, &reference, GROUP, &specs)?;
        for case in live {
            let passed = case.outcome == GateOutcome::Pass;
            if disputed.contains(&case.id) && passed != eligible.contains(&case.id) {
                *control.losses.entry(case.id).or_default() += 1;
            }
        }
    }
    Ok(control)
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
struct Control {
    runs: usize,
    losses: BTreeMap<String, usize>,
}

impl Control {
    fn contended(&self) -> BTreeSet<String> {
        self.losses.keys().cloned().collect()
    }
}

/// How many control runs the disputed files are worth.
///
/// The budget is one group run. A control that costs more than the gate
/// it is checking would be paid on every red run, including the ones
/// where the failure is plainly not a race.
fn control_runs(manifest: &crate::OilsManifest, specs: &BTreeSet<String>) -> usize {
    let mut group_cases = 0_usize;
    let mut selected_cases = 0_usize;
    for entry in &manifest.specs {
        if !entry.groups.iter().any(|group| group == GROUP) {
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

// [spec:nsh:req:compat.bash.survey-closure]
fn gate(root: &Path, shell: &Path) -> Result<bool> {
    /* Resolve first, then insist on the name. The runner canonicalizes
     * the shell before running it, so a symlink called `bash` pointing at
     * some other file is executed under that file's name -- and nsh reads
     * its own name to decide whether Bash mode is on. Checking the name
     * given rather than the name run let a link called `bash` score nsh
     * in POSIX mode against a Bash suite, which is where the 793 in the
     * log came from. */
    // [spec:nsh:req:compat.bash.survey-closure]
    let shell = fs::canonicalize(shell)?;
    require_bash_basename(&shell)?;
    let shell = shell.as_path();
    let root = fs::canonicalize(root)?;
    let register = read_register(&root)?;
    let lock = crate::read_lock(&root)?;
    crate::verify_import(&root, &lock)?;
    crate::verify_oils_manifest(&root, &lock)?;
    if register.oils_commit != lock.commit {
        return Err(format!(
            "{REGISTER_FILE} pins Oils {}, the corpus is {}",
            register.oils_commit, lock.commit
        )
        .into());
    }
    let manifest: crate::OilsManifest =
        toml::from_str(&fs::read_to_string(root.join("MANIFEST.toml"))?)?;
    let (eligible, reference_excluded) = crate::bash_reference::calibration(&root)?;

    let observed = run_gate_group(&root, &manifest, shell, GROUP)?;
    let mut findings = judge(
        &register,
        &observed,
        &eligible,
        &reference_excluded,
        &BTreeSet::new(),
    );
    /* The control is only paid for when there is something to decide. A
     * clean gate is the common case and is exactly as fast as it was. */
    let mut control = Control::default();
    if !findings.disputed.is_empty() {
        control = contended_cases(&root, &manifest, &eligible, &findings.disputed)?;
        findings = judge(
            &register,
            &observed,
            &eligible,
            &reference_excluded,
            &control.contended(),
        );
    }
    report(&register, &observed, &findings, &control);
    Ok(findings.violations.is_empty())
}

fn judge(
    register: &Register,
    observed: &[GateCase],
    eligible: &BTreeSet<String>,
    reference_excluded: &BTreeMap<String, String>,
    contended: &BTreeSet<String>,
) -> Findings {
    let out_of_scope: BTreeSet<&str> = register
        .scope
        .iter()
        .map(|entry| entry.spec.as_str())
        .collect();
    let registered: BTreeMap<&str, &CaseEntry> = register
        .cases
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut findings = Findings {
        violations: Vec::new(),
        disputed: BTreeSet::new(),
        passing: 0,
        counts: BTreeMap::new(),
        out_of_contract: 0,
        reference_excluded: BTreeMap::new(),
        contended: BTreeSet::new(),
    };

    let specs: BTreeSet<&str> = observed.iter().map(|case| case.spec.as_str()).collect();
    for entry in &register.scope {
        if !specs.contains(entry.spec.as_str()) {
            findings.violations.push(format!(
                "scope entry {} names no spec in the group",
                entry.spec
            ));
        }
    }

    let known: BTreeSet<&str> = observed.iter().map(|case| case.id.as_str()).collect();
    for entry in &register.cases {
        if !known.contains(entry.id.as_str()) {
            findings
                .violations
                .push(format!("{} is not a case in the group", entry.id));
        }
    }

    for case in observed {
        if out_of_scope.contains(case.spec.as_str()) {
            findings.out_of_contract += 1;
            continue;
        }
        if contended.contains(&case.id) {
            findings.contended.insert(case.id.clone());
            continue;
        }
        judge_in_scope_case(
            case,
            eligible,
            reference_excluded,
            &registered,
            &mut findings,
        );
    }
    findings
}

fn judge_in_scope_case(
    case: &GateCase,
    eligible: &BTreeSet<String>,
    reference_excluded: &BTreeMap<String, String>,
    registered: &BTreeMap<&str, &CaseEntry>,
    findings: &mut Findings,
) {
    let entry = registered.get(case.id.as_str());
    if !eligible.contains(&case.id) {
        if let Some(entry) = entry {
            findings.violations.push(format!(
                "{} is registered as {} but the reference calibration already excludes it",
                case.id,
                entry.disposition.label()
            ));
        }
        match reference_excluded.get(&case.id) {
            Some(disposition) => {
                *findings
                    .reference_excluded
                    .entry(disposition.clone())
                    .or_default() += 1
            }
            None => findings.violations.push(format!(
                "{} is outside the eligible manifest with no recorded disposition",
                case.id
            )),
        }
        return;
    }

    if matches!(case.outcome, GateOutcome::Timeout | GateOutcome::Error) {
        findings.disputed.insert(case.id.clone());
        findings.violations.push(format!(
            "{} ended as {} -- a timeout or a harness error is never expected",
            case.id,
            case.outcome.label()
        ));
        return;
    }

    match (case.outcome, entry) {
        (GateOutcome::Pass, None) => findings.passing += 1,
        (GateOutcome::Pass, Some(entry)) => {
            findings.disputed.insert(case.id.clone());
            findings.violations.push(format!(
                "{} passes but is still registered as {}; remove the stale entry",
                case.id,
                entry.disposition.label()
            ));
        }
        (_, Some(entry)) => *findings.counts.entry(entry.disposition).or_default() += 1,
        (outcome, None) => {
            findings.disputed.insert(case.id.clone());
            findings.violations.push(format!(
                "{} is an unexpected {} in the eligible manifest",
                case.id,
                outcome.label()
            ));
        }
    }
}

fn report(register: &Register, observed: &[GateCase], findings: &Findings, control: &Control) {
    let in_scope = observed.len() - findings.out_of_contract;
    let non_passing: usize = findings.counts.values().sum();
    let reference_total: usize = findings.reference_excluded.values().sum();
    println!("Bash compatibility closure gate");
    println!("group: {}", register.group);
    println!("cases in group: {}", observed.len());
    println!(
        "out of contract: {} (whole files, {} entries)",
        findings.out_of_contract,
        register.scope.len()
    );
    println!("in scope: {in_scope}");
    println!("outside the eligible manifest: {reference_total}");
    for (disposition, count) in &findings.reference_excluded {
        println!("  {disposition}: {count}");
    }
    println!(
        "eligible manifest: {} ({} pass, {non_passing} dispositioned)",
        findings.passing + non_passing,
        findings.passing
    );
    for (category, count) in &findings.counts {
        println!("  {}: {count}", category.label());
    }
    if !findings.contended.is_empty() {
        println!(
            "undecided this run: {} -- the pinned Bash did not reproduce its own \
             recorded result on this machine, so these measure the machine rather \
             than the shell",
            findings.contended.len()
        );
        for id in &findings.contended {
            let losses = control.losses.get(id).copied().unwrap_or_default();
            println!(
                "  {id} -- the pinned Bash lost it in {losses} of {} control runs",
                control.runs
            );
        }
    }
    if findings.violations.is_empty() {
        println!("gate: PASS -- no unexpected failure, timeout or harness error");
        return;
    }
    println!("gate: FAIL -- {} violations", findings.violations.len());
    for violation in &findings.violations {
        println!("  {violation}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/surveys/oils")
    }

    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn register_parses_and_names_cases() {
        let register = read_register(&root()).unwrap();
        assert!(!register.cases.is_empty());
        assert_eq!(register.scope.len(), 6);
        let mut seen = BTreeSet::new();
        for entry in &register.cases {
            assert!(seen.insert(entry.id.clone()), "duplicate {}", entry.id);
            let (spec, index) = entry.id.rsplit_once(':').expect("spec:index");
            assert!(spec.ends_with(".test.sh"), "{}", entry.id);
            index.parse::<usize>().expect("numeric index");
        }
    }

    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn registered_cases_exist_in_corpus() {
        let root = root();
        let lock = crate::read_lock(&root).unwrap();
        let manifest = toml::from_str::<crate::OilsManifest>(
            &fs::read_to_string(root.join("MANIFEST.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.source_commit, lock.commit);
        let catalog = crate::oils_runner::bash_case_catalog(&root, &manifest).unwrap();
        let known: BTreeSet<String> = catalog
            .into_iter()
            .filter(|case| case.groups.iter().any(|group| group == GROUP))
            .map(|case| case.id)
            .collect();
        let register = read_register(&root).unwrap();
        for entry in &register.cases {
            assert!(known.contains(&entry.id), "unknown case {}", entry.id);
        }
        for entry in &register.scope {
            assert!(
                known.iter().any(|id| id.starts_with(&entry.spec)),
                "unknown spec {}",
                entry.spec
            );
        }
    }

    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn registered_cases_are_eligible() {
        let root = root();
        let (eligible, excluded) = crate::bash_reference::calibration(&root).unwrap();
        let register = read_register(&root).unwrap();
        for entry in &register.cases {
            assert!(
                eligible.contains(&entry.id),
                "{} is not eligible; the reference already excludes it as {:?}",
                entry.id,
                excluded.get(&entry.id)
            );
        }
    }

    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn another_shell_name_is_refused() {
        assert!(require_bash_basename(Path::new("/tmp/bash")).is_ok());
        assert!(require_bash_basename(Path::new("/tmp/bash-pre")).is_err());
        assert!(require_bash_basename(Path::new("/tmp/nsh")).is_err());
    }
    /// The gate's verdict has to depend on what the shell did.
    ///
    /// This is the demonstration that the oracle can fail, and it is the
    /// deliverable rather than any number the gate produces afterwards. A
    /// gate that scores every shell alike certifies
    /// `[spec:nsh:req:compat.bash.survey-closure]` without measuring
    /// anything, which is the same defect this repository spent seven
    /// nodes removing from the round-trip oracle -- there a fixed point
    /// any consistent output satisfied, here a gate any shell satisfies.
    ///
    /// Judged over the real register and the real eligible set, so it is
    /// the shipped judgement being asked and not a model of it. The two
    /// observation sets are the two extremes a shell can produce: every
    /// eligible case passing, and every one failing.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn the_verdict_depends_on_what_the_shell_did() {
        let root = root();
        let (eligible, excluded) = crate::bash_reference::calibration(&root).unwrap();
        let register = read_register(&root).unwrap();
        let observe = |outcome: GateOutcome| -> Vec<GateCase> {
            eligible
                .iter()
                .map(|id| GateCase {
                    id: id.clone(),
                    spec: id.rsplit_once(':').expect("spec:index").0.to_owned(),
                    outcome,
                })
                .collect()
        };
        let quiet = BTreeSet::new();
        let passing = judge(
            &register,
            &observe(GateOutcome::Pass),
            &eligible,
            &excluded,
            &quiet,
        );
        let failing = judge(
            &register,
            &observe(GateOutcome::Fail),
            &eligible,
            &excluded,
            &quiet,
        );
        let unexpected = |findings: &Findings| {
            findings
                .violations
                .iter()
                .filter(|violation| violation.contains("is an unexpected failure"))
                .count()
        };
        assert_eq!(
            unexpected(&passing),
            0,
            "a shell that passes every case cannot have an unexpected failure",
        );
        assert!(
            unexpected(&failing) > 800,
            "a shell that fails every eligible case must be seen to fail: {} unexpected",
            unexpected(&failing),
        );
        /* A case the reference did not reproduce this run is not evidence
         * about the shell, and the gate must be seen to stop counting it
         * rather than merely to say so. */
        let one: BTreeSet<String> = eligible.iter().take(1).cloned().collect();
        let contended = judge(
            &register,
            &observe(GateOutcome::Fail),
            &eligible,
            &excluded,
            &one,
        );
        assert_eq!(
            unexpected(&contended),
            unexpected(&failing) - 1,
            "marking one case undecided did not take it out of the verdict",
        );
        assert_eq!(contended.contended, one);

        assert_ne!(
            passing.violations.len(),
            failing.violations.len(),
            "the gate reported the same verdict for a shell that passed everything \
             and one that failed everything",
        );
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
        let root = root();
        let manifest = toml::from_str::<crate::OilsManifest>(
            &fs::read_to_string(root.join("MANIFEST.toml")).unwrap(),
        )
        .unwrap();

        let one: BTreeSet<String> = ["process-sub.test.sh".to_owned()].into_iter().collect();
        assert_eq!(
            control_runs(&manifest, &one),
            CONTROL_RUNS,
            "one nine-case file did not earn every control run",
        );

        let whole: BTreeSet<String> = manifest
            .specs
            .iter()
            .filter(|entry| entry.groups.iter().any(|group| group == GROUP))
            .map(|entry| {
                Path::new(&entry.path)
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            control_runs(&manifest, &whole),
            1,
            "a dispute covering the group cost more than one extra group run",
        );

        assert_eq!(
            control_runs(&manifest, &BTreeSet::new()),
            0,
            "nothing disputed still bought a control run",
        );
    }

    /// Only a live outcome can be disputed.
    ///
    /// The control re-runs the reference, which can change what a case
    /// does and cannot change what three checked-in files say. A register
    /// entry naming a case that is not in the group is the second kind,
    /// and asking the reference about it would spend the budget on a
    /// question no run can answer.
    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn only_a_live_outcome_is_disputed() {
        let root = root();
        let register = read_register(&root).unwrap();
        let (eligible, excluded) = crate::bash_reference::calibration(&root).unwrap();
        let observed: Vec<GateCase> = eligible
            .iter()
            .take(3)
            .map(|id| GateCase {
                id: id.clone(),
                spec: id.rsplit_once(':').unwrap().0.to_owned(),
                outcome: GateOutcome::Fail,
            })
            .collect();
        let findings = judge(&register, &observed, &eligible, &excluded, &BTreeSet::new());
        let live: BTreeSet<String> = observed.iter().map(|case| case.id.clone()).collect();
        assert_eq!(
            findings.disputed, live,
            "the three failing cases were not the three disputed ones",
        );
        /* Every registered case is missing from `observed`, so the
         * register raises a violation for each -- and not one of them is
         * disputed, because no run of anything decides it. */
        assert!(
            findings.violations.len() > findings.disputed.len(),
            "the register's own violations were counted as disputes",
        );
    }

    /// The name that matters is the name the shell runs under.
    ///
    /// The runner canonicalizes the shell before executing it, and nsh
    /// reads its own name to decide whether Bash mode is on. So a link
    /// called `bash` pointing at `nsh` satisfies a check on the name
    /// given and then runs with the dialect off, scoring a POSIX shell
    /// against a Bash suite. That is where the 793 in this node's log
    /// came from: 80 of 873 eligible cases passing, reported as a
    /// measurement of Bash compatibility.
    // [spec:nsh:req:compat.bash.survey-closure/test]
    #[test]
    fn a_link_named_bash_over_another_shell_fails() {
        let scratch =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/nsh-survey-gate-name");
        drop(fs::remove_dir_all(&scratch));
        fs::create_dir_all(&scratch).unwrap();
        let target = scratch.join("nsh");
        fs::write(&target, b"#!/bin/sh\nexit 0\n").unwrap();
        let link = scratch.join("bash");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // The name as given is `bash`, which is what used to be checked.
        assert!(require_bash_basename(&link).is_ok());
        // The name it resolves to is not, and that is the one that runs.
        let resolved = fs::canonicalize(&link).unwrap();
        assert!(
            require_bash_basename(&resolved).is_err(),
            "a link called bash resolving to {} was accepted",
            resolved.display(),
        );
        drop(fs::remove_dir_all(&scratch));
    }
}
