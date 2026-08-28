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

use crate::oils_runner::{GateCase, GateOutcome, run_gate_group};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const REGISTER_FILE: &str = "BASH_DISPOSITIONS.toml";
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
    passing: usize,
    counts: BTreeMap<Category, usize>,
    out_of_contract: usize,
    reference_excluded: BTreeMap<String, usize>,
}

// [spec:nsh:req:compat.bash.survey-closure]
fn gate(root: &Path, shell: &Path) -> Result<bool> {
    require_bash_basename(shell)?;
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
    let findings = judge(&register, &observed, &eligible, &reference_excluded);
    report(&register, &observed, &findings);
    Ok(findings.violations.is_empty())
}

fn judge(
    register: &Register,
    observed: &[GateCase],
    eligible: &BTreeSet<String>,
    reference_excluded: &BTreeMap<String, String>,
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
        passing: 0,
        counts: BTreeMap::new(),
        out_of_contract: 0,
        reference_excluded: BTreeMap::new(),
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
        findings.violations.push(format!(
            "{} ended as {} -- a timeout or a harness error is never expected",
            case.id,
            case.outcome.label()
        ));
        return;
    }

    match (case.outcome, entry) {
        (GateOutcome::Pass, None) => findings.passing += 1,
        (GateOutcome::Pass, Some(entry)) => findings.violations.push(format!(
            "{} passes but is still registered as {}; remove the stale entry",
            case.id,
            entry.disposition.label()
        )),
        (_, Some(entry)) => *findings.counts.entry(entry.disposition).or_default() += 1,
        (outcome, None) => findings.violations.push(format!(
            "{} is an unexpected {} in the eligible manifest",
            case.id,
            outcome.label()
        )),
    }
}

fn report(register: &Register, observed: &[GateCase], findings: &Findings) {
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
        let passing = judge(&register, &observe(GateOutcome::Pass), &eligible, &excluded);
        let failing = judge(&register, &observe(GateOutcome::Fail), &eligible, &excluded);
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
        assert_ne!(
            passing.violations.len(),
            failing.violations.len(),
            "the gate reported the same verdict for a shell that passed everything \
             and one that failed everything",
        );
    }
}
