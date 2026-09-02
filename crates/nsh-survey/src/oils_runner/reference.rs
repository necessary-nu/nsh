use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceOutcome {
    Pass,
    Fail,
    Unsupported,
    KnownBug,
    Timeout,
    Error,
}

#[derive(Debug)]
pub(crate) struct ReferenceCase {
    pub(crate) id: String,
    pub(crate) spec: String,
    pub(crate) index: usize,
    pub(crate) line: usize,
    pub(crate) description: String,
    pub(crate) outcome: ReferenceOutcome,
    pub(crate) status: Option<i32>,
    pub(crate) qualifier: Option<String>,
    pub(crate) difference_fields: Vec<String>,
    pub(crate) note: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReferenceTotals {
    pub(crate) selected: usize,
    pub(crate) executed: usize,
    pub(crate) pass: usize,
    pub(crate) fail: usize,
    pub(crate) unsupported: usize,
    pub(crate) known_bug: usize,
    pub(crate) timeout: usize,
    pub(crate) error: usize,
}

#[derive(Debug)]
pub(crate) struct ReferenceReport {
    pub(crate) source_commit: String,
    pub(crate) group: String,
    pub(crate) shell_sha256: String,
    pub(crate) containment: String,
    pub(crate) timeout_ms: u64,
    pub(crate) totals: ReferenceTotals,
    pub(crate) cases: Vec<ReferenceCase>,
}

#[derive(Debug)]
pub(crate) struct CatalogCase {
    pub(crate) id: String,
    pub(crate) spec: String,
    pub(crate) index: usize,
    pub(crate) line: usize,
    pub(crate) description: String,
    pub(crate) groups: Vec<String>,
    pub(crate) version_specific_bash: bool,
}

pub(crate) fn run_reference_group(
    root: &Path,
    manifest: &crate::OilsManifest,
    shell: &Path,
    group: &str,
) -> Result<ReferenceReport> {
    run_reference_specs(root, manifest, shell, group, &BTreeSet::new())
}

/// Run a group, or the named spec files within it, in the reference
/// environment.
///
/// An empty `specs` is the whole group and insists that nothing was
/// skipped -- a calibration that silently measured a subset would be
/// worth less than no calibration. A non-empty one is a deliberate
/// subset, so the skips are the point rather than a fault.
pub(crate) fn run_reference_specs(
    root: &Path,
    manifest: &crate::OilsManifest,
    shell: &Path,
    group: &str,
    specs: &BTreeSet<String>,
) -> Result<ReferenceReport> {
    let root = fs::canonicalize(root)?;
    let shell = fs::canonicalize(shell)?;
    let whole_group = specs.is_empty();
    let options = Options {
        root,
        group: group.to_owned(),
        reported_shell: display_shell(&shell),
        shell,
        expectation_shell: "bash".to_owned(),
        timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        format: OutputFormat::Text,
        specs: specs.clone(),
        case_filter: None,
        max_cases: None,
        summary: None,
        baseline: None,
        refresh: super::Refresh::No,
        overwrite: false,
        posix: false,
        verbose: false,
        base_path: Some(OsString::from("/usr/bin:/bin")),
        timezone: Some(OsString::from("UTC")),
        locale_archive: Some(OsString::new()),
    };
    let report = run_manifest(&options, manifest)?;
    if whole_group && report.totals.skip != 0 {
        return Err(format!(
            "reference group {} unexpectedly skipped {} cases",
            report.group, report.totals.skip
        )
        .into());
    }
    Ok(ReferenceReport {
        source_commit: report.source_commit,
        group: report.group,
        shell_sha256: report.shell_sha256,
        containment: report.containment,
        timeout_ms: report.timeout_ms,
        totals: ReferenceTotals {
            selected: report.totals.selected,
            executed: report.totals.executed,
            pass: report.totals.pass,
            fail: report.totals.fail,
            unsupported: report.totals.unsupported,
            known_bug: report.totals.known_bug,
            timeout: report.totals.timeout,
            error: report.totals.error,
        },
        cases: report
            .cases
            .into_iter()
            .map(|case| ReferenceCase {
                id: format!("{}:{}", case.spec, case.index),
                spec: case.spec,
                index: case.index,
                line: case.line,
                description: case.description,
                outcome: match case.outcome {
                    Outcome::Pass => ReferenceOutcome::Pass,
                    Outcome::Fail => ReferenceOutcome::Fail,
                    Outcome::Unsupported => ReferenceOutcome::Unsupported,
                    Outcome::KnownBug => ReferenceOutcome::KnownBug,
                    Outcome::Timeout => ReferenceOutcome::Timeout,
                    Outcome::Error => ReferenceOutcome::Error,
                    Outcome::Skip => unreachable!("skip count was checked above"),
                },
                status: case.status,
                qualifier: case.qualifier,
                difference_fields: case
                    .differences
                    .into_iter()
                    .map(|difference| difference.field)
                    .collect(),
                note: case.note,
            })
            .collect(),
    })
}

pub(crate) fn bash_case_catalog(
    root: &Path,
    manifest: &crate::OilsManifest,
) -> Result<Vec<CatalogCase>> {
    let mut catalog = Vec::new();
    for entry in &manifest.specs {
        if !entry.groups.iter().any(|group| group == "bash-comparison") {
            continue;
        }
        let parsed = parse_spec(&root.join(&entry.path))?;
        if parsed.cases.len() != entry.cases {
            return Err(format!(
                "{} parsed as {} cases, manifest records {}",
                entry.path,
                parsed.cases.len(),
                entry.cases
            )
            .into());
        }
        let spec = Path::new(&entry.path)
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("non-UTF-8 manifest path {}", entry.path))?;
        let bash_tokens = entry
            .compare_shells
            .iter()
            .filter(|shell| shell.starts_with("bash"))
            .collect::<Vec<_>>();
        let version_specific_bash =
            !bash_tokens.is_empty() && bash_tokens.iter().all(|shell| shell.as_str() != "bash");
        let groups = entry
            .groups
            .iter()
            .filter(|group| {
                matches!(
                    group.as_str(),
                    "bash-comparison" | "bash-extension" | "bash-named-diagnostic"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        for (index, case) in parsed.cases.into_iter().enumerate() {
            catalog.push(CatalogCase {
                id: format!("{spec}:{index}"),
                spec: spec.to_owned(),
                index,
                line: case.line,
                description: case.description,
                groups: groups.clone(),
                version_specific_bash,
            });
        }
    }
    Ok(catalog)
}

/// One case as the closure gate sees it: identity and outcome, with the
/// difference detail dropped -- the gate decides on the category, not on
/// the bytes.
#[derive(Debug)]
pub(crate) struct GateCase {
    pub(crate) id: String,
    pub(crate) spec: String,
    pub(crate) outcome: GateOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GateOutcome {
    Pass,
    Fail,
    Unsupported,
    KnownBug,
    Timeout,
    Error,
}

impl GateOutcome {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "failure",
            Self::Unsupported => "unsupported result",
            Self::KnownBug => "known Bash defect",
            Self::Timeout => "timeout",
            Self::Error => "harness error",
        }
    }
}

/// Run one group for the gate, with the same containment and the same
/// fixed environment the reference calibration used.
// [spec:nsh:req:compat.bash.survey-closure]
pub(crate) fn run_gate_group(
    root: &Path,
    manifest: &crate::OilsManifest,
    shell: &Path,
    group: &str,
) -> Result<Vec<GateCase>> {
    run_gate_specs(root, manifest, shell, group, &BTreeSet::new())
}

/// The same run, narrowed to the spec files named.
///
/// The control run needs this: it re-asks the reference about the cases
/// a verdict turned on, and re-running the whole group to ask about one
/// file buys three samples where the question needs a dozen.
// [spec:nsh:req:compat.bash.survey-closure]
pub(crate) fn run_gate_specs(
    root: &Path,
    manifest: &crate::OilsManifest,
    shell: &Path,
    group: &str,
    specs: &BTreeSet<String>,
) -> Result<Vec<GateCase>> {
    let report = run_reference_specs(root, manifest, shell, group, specs)?;
    Ok(report
        .cases
        .into_iter()
        .map(|case| GateCase {
            id: case.id,
            spec: case.spec,
            outcome: match case.outcome {
                ReferenceOutcome::Pass => GateOutcome::Pass,
                ReferenceOutcome::Fail => GateOutcome::Fail,
                ReferenceOutcome::Unsupported => GateOutcome::Unsupported,
                ReferenceOutcome::KnownBug => GateOutcome::KnownBug,
                ReferenceOutcome::Timeout => GateOutcome::Timeout,
                ReferenceOutcome::Error => GateOutcome::Error,
            },
        })
        .collect())
}
