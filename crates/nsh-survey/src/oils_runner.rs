use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::process::{
    Containment, OUTPUT_LIMIT, Output as ProcessResult, Request as ProcessRequest, ScratchTree,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

mod baseline;
pub(crate) mod helpers;
mod reference;

pub(crate) use reference::{
    CatalogCase, GateCase, GateOutcome, ReferenceCase, ReferenceOutcome, ReferenceReport,
    ReferenceTotals, bash_case_catalog, run_gate_group, run_gate_specs, run_reference_group,
};

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

pub(crate) fn command(args: env::ArgsOs, default_root: PathBuf) -> Result<bool> {
    let Some(mut options) = Options::parse(args, default_root)? else {
        println!("{}", Options::usage());
        return Ok(true);
    };
    /* SAID RATHER THAN REFUSED, which is the difference between this
     * command and the gate. `run-oils` scores whatever it is pointed at,
     * including the pinned Bash and shells from outside this tree, so a
     * stale build is a fact about the run to put in front of the reader
     * rather than a verdict. The gate scores this repository's shell and
     * nothing else, so there it is a refusal. */
    if let Some(complaint) = crate::shell::built_before_its_sources(&options.shell)? {
        eprintln!("warning: {complaint}");
    }
    /* THE RUN INSTALLS ITS OWN SHELL. `target/gate/bash` was the
     * documented path and it is a shared mutable file: another session's
     * build replaced it between two runs a minute apart, and the two runs
     * disagreed about which cases were failing. There is no shared path
     * to collide on now, and the name is derived from the expectation
     * namespace rather than left to a `cp` the reader has to remember. */
    let installed = crate::shell::ShellUnderTest::install(
        &options.shell,
        &crate::shell::name_for(&options.expectation_shell, &options.shell),
    )?;
    options.shell = installed.path();
    crate::read_lock(&options.root).and_then(|lock| {
        crate::verify_import(&options.root, &lock)?;
        crate::verify_oils_manifest(&options.root, &lock)
    })?;
    let manifest: crate::OilsManifest =
        toml::from_str(&fs::read_to_string(options.root.join("MANIFEST.toml"))?)?;
    /* Asked before the run rather than after it. The binary was built
     * before the run started, so the tree as it stands now is the closest
     * reading there is of what went into it -- and a refusal costs a
     * second here instead of the group run it would otherwise waste. */
    let provenance = match (&options.baseline, options.refresh) {
        (Some(path), Refresh::FromCommitted) => Some(crate::provenance::vouch(path, false)?),
        (Some(path), Refresh::FromDirtyTree) => Some(crate::provenance::vouch(path, true)?),
        _ => None,
    };
    /* And the second question about the same file: not whose work is in
     * the shell, but whose work is in the file this run is about to
     * replace. Asked here so a refusal costs a second rather than the
     * group run, and again at the write, because the group run is exactly
     * the window another session re-records in. */
    if let (Some(path), true) = (&options.baseline, provenance.is_some()) {
        crate::provenance::guard_generated(path, options.overwrite)?;
    }
    let report = run_manifest(&options, &manifest)?;
    if let Some(path) = &options.summary {
        write_summary(path, &report)?;
    }
    match options.format {
        OutputFormat::Text => report.write_text(options.verbose),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Ids => report.write_ids(),
    }
    /* With a baseline the run's verdict is the comparison, not the score.
     * A group like `bash-comparison` has hundreds of expected failures and
     * so is never `is_success`; without this it could not be a check at
     * all, and reading its summary count instead is the mistake the
     * baseline exists to retire. */
    match &options.baseline {
        Some(path) => baseline::apply(
            &report,
            &manifest,
            &options.root,
            path,
            provenance,
            options.overwrite,
        ),
        None => Ok(report.totals.is_success()),
    }
}

#[derive(Debug)]
struct Options {
    root: PathBuf,
    group: String,
    shell: PathBuf,
    /// How the shell is named in the report.
    ///
    /// `shell` is the private copy this run installed, whose path is
    /// unique to the run; a summary that recorded it would carry a
    /// different string every time it was regenerated and name a
    /// directory that no longer exists. What a reader wants is the binary
    /// the caller pointed at, and `shell_sha256` beside it says which
    /// bytes those were.
    reported_shell: String,
    expectation_shell: String,
    timeout: Duration,
    format: OutputFormat,
    specs: BTreeSet<String>,
    case_filter: Option<String>,
    max_cases: Option<usize>,
    summary: Option<PathBuf>,
    baseline: Option<PathBuf>,
    refresh: Refresh,
    overwrite: bool,
    posix: bool,
    verbose: bool,
    base_path: Option<OsString>,
    timezone: Option<OsString>,
    locale_archive: Option<OsString>,
}

/// What the run was asked to do with the failing-case list.
///
/// `FromDirtyTree` is spelled `--update-baseline-from-dirty-tree` because
/// what it waives is the question of whose work is in the shell being
/// measured. See `crate::provenance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refresh {
    /// Compare against the recorded list.
    No,
    /// Re-record it, refusing a checkout that cannot vouch for the shell.
    FromCommitted,
    /// Re-record it anyway, naming the uncommitted paths inside the file.
    FromDirtyTree,
}

#[derive(Clone, Copy, Debug)]
enum OutputFormat {
    Text,
    Json,
    /// Just the failing case ids, one per line, sorted.
    ///
    /// The whole reason this exists is that the ids used to be recovered
    /// from the text report with a regular expression, and the one in use
    /// dropped every spec whose name contains an underscore.
    Ids,
}

impl Options {
    fn parse(mut args: env::ArgsOs, default_root: PathBuf) -> Result<Option<Self>> {
        let mut options = Self {
            root: default_root,
            group: "full".to_owned(),
            shell: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/nsh"),
            reported_shell: String::new(),
            expectation_shell: "osh".to_owned(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            format: OutputFormat::Text,
            specs: BTreeSet::new(),
            case_filter: None,
            max_cases: None,
            summary: None,
            baseline: None,
            refresh: Refresh::No,
            overwrite: false,
            posix: false,
            verbose: false,
            base_path: None,
            timezone: None,
            locale_archive: None,
        };
        let mut root_seen = false;
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("-h" | "--help") => return Ok(None),
                Some("--group") => options.group = required_string(&mut args, "--group")?,
                Some("--shell") => {
                    options.shell = args
                        .next()
                        .map(PathBuf::from)
                        .ok_or("--shell requires a path")?
                }
                Some("--expect-shell") => {
                    options.expectation_shell = required_string(&mut args, "--expect-shell")?
                }
                Some("--timeout-ms") => {
                    let value = required_string(&mut args, "--timeout-ms")?;
                    let milliseconds: u64 = value.parse()?;
                    if milliseconds == 0 || milliseconds > 600_000 {
                        return Err("--timeout-ms must be between 1 and 600000".into());
                    }
                    options.timeout = Duration::from_millis(milliseconds);
                }
                Some("--format") => {
                    options.format = match required_string(&mut args, "--format")?.as_str() {
                        "text" => OutputFormat::Text,
                        "json" => OutputFormat::Json,
                        "ids" => OutputFormat::Ids,
                        value => return Err(format!("unsupported output format {value:?}").into()),
                    }
                }
                Some("--spec") => {
                    options.specs.insert(required_string(&mut args, "--spec")?);
                }
                Some("--case") => {
                    options.case_filter = Some(required_string(&mut args, "--case")?.to_lowercase())
                }
                Some("--max-cases") => {
                    options.max_cases = Some(required_string(&mut args, "--max-cases")?.parse()?);
                }
                Some("--summary") => options.summary = Some(required_path(&mut args, "--summary")?),
                Some("--baseline") => {
                    options.baseline = Some(required_path(&mut args, "--baseline")?)
                }
                Some("--update-baseline") => options.refresh = Refresh::FromCommitted,
                Some("--update-baseline-from-dirty-tree") => {
                    options.refresh = Refresh::FromDirtyTree
                }
                Some("--overwrite-a-changed-file") => options.overwrite = true,
                Some("--posix") => options.posix = true,
                Some("--verbose") => options.verbose = true,
                Some(value) if value.starts_with('-') => {
                    return Err(
                        format!("unknown run-oils option {value:?}; {}", Self::usage()).into(),
                    );
                }
                _ if !root_seen => {
                    options.root = PathBuf::from(argument);
                    root_seen = true;
                }
                _ => {
                    return Err(
                        format!("unexpected argument {argument:?}; {}", Self::usage()).into(),
                    );
                }
            }
        }
        options.root = fs::canonicalize(&options.root).map_err(|error| {
            format!(
                "cannot resolve survey root {}: {error}",
                options.root.display()
            )
        })?;
        options.shell = fs::canonicalize(&options.shell).map_err(|error| {
            format!(
                "cannot resolve shell {}: {error}; build it with cargo build --release -p nsh-cli",
                options.shell.display()
            )
        })?;
        options.reported_shell = display_shell(&options.shell);
        options.check_baseline_is_answerable()?;
        Ok(Some(options))
    }

    /// Refuse a baseline comparison that cannot mean what it says.
    ///
    /// A baseline is the failing-case list of a whole group. Compare a
    /// filtered run against one and every unselected case reads as
    /// fixed -- a mismatch of hundreds that says nothing, or worse, a
    /// filtered *re-record* that quietly shrinks the list to whatever
    /// the filter happened to select. Neither is a comparison, so
    /// neither is allowed to look like one.
    fn check_baseline_is_answerable(&self) -> Result<()> {
        if self.refresh != Refresh::No && self.baseline.is_none() {
            return Err("--update-baseline needs --baseline PATH to write".into());
        }
        if self.baseline.is_none() {
            return Ok(());
        }
        let filters = [
            ("--spec", !self.specs.is_empty()),
            ("--case", self.case_filter.is_some()),
            ("--max-cases", self.max_cases.is_some()),
        ];
        for (option, given) in filters {
            if given {
                return Err(format!(
                    "--baseline covers a whole group and {option} selects part of one; \
                     every unselected case would read as fixed"
                )
                .into());
            }
        }
        Ok(())
    }

    fn usage() -> &'static str {
        "usage: nsh-survey run-oils [--group ID] [--shell PATH] [--expect-shell LABEL]\n\
                [--timeout-ms N] [--format text|json|ids] [--spec NAME] [--case TEXT]\n\
                [--max-cases N] [--summary PATH] [--baseline PATH] [--update-baseline]\n\
                [--update-baseline-from-dirty-tree] [--overwrite-a-changed-file]\n\
                [--posix] [--verbose] [ROOT]"
    }
}

fn required_string(args: &mut env::ArgsOs, option: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))?
        .into_string()
        .map_err(|_| format!("{option} requires UTF-8 text").into())
}

fn required_path(args: &mut env::ArgsOs, option: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{option} requires a path").into())
}

fn run_manifest(options: &Options, manifest: &crate::OilsManifest) -> Result<RunReport> {
    let group = manifest
        .groups
        .iter()
        .find(|candidate| candidate.id == options.group)
        .ok_or_else(|| {
            let choices = manifest
                .groups
                .iter()
                .map(|group| group.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "unknown Oils group {:?}; choose one of {choices}",
                options.group
            )
        })?;
    let scratch = ScratchTree::new()?;
    let containment = Containment::verified(scratch.path())?;
    containment.verify_reaches(
        scratch.path(),
        "the shell under test",
        &options.shell,
        "give a path outside /tmp.",
    )?;
    let fixture_view = helpers::install(scratch.path(), &options.root)?;
    let inherited_path = options
        .base_path
        .clone()
        .unwrap_or_else(|| env::var_os("PATH").unwrap_or_default());
    let mut path_parts = vec![fixture_view.bin.clone()];
    path_parts.extend(env::split_paths(&inherited_path));
    let survey_path = env::join_paths(path_parts)?;
    let context = RunContext {
        root: &fixture_view.root,
        shell: &options.shell,
        expectation_shell: &options.expectation_shell,
        timeout: options.timeout,
        posix: options.posix,
        survey_path,
        scratch: scratch.path(),
        containment: &containment,
        timezone: options.timezone.as_deref(),
        locale_archive: options.locale_archive.as_deref(),
    };

    let started = Instant::now();
    let mut records = Vec::with_capacity(group.cases);
    let mut totals = Totals {
        selected: group.cases,
        ..Totals::default()
    };
    let mut eligible = 0_usize;
    let mut parsed_files = 0_usize;
    let mut parsed_cases = 0_usize;
    for entry in manifest
        .specs
        .iter()
        .filter(|entry| entry.groups.iter().any(|candidate| candidate == &group.id))
    {
        let path = options.root.join(&entry.path);
        let parsed = parse_spec(&path)?;
        if parsed.cases.len() != entry.cases {
            return Err(format!(
                "{} parsed as {} cases, manifest records {}",
                entry.path,
                parsed.cases.len(),
                entry.cases
            )
            .into());
        }
        parsed_files += 1;
        parsed_cases += parsed.cases.len();
        let spec_name = Path::new(&entry.path)
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("non-UTF-8 manifest path {}", entry.path))?;
        let spec_selected = options.specs.is_empty()
            || options
                .specs
                .iter()
                .any(|filter| matches_spec(filter, &entry.path, spec_name));
        for (index, case) in parsed.cases.iter().enumerate() {
            let case_selected = options.case_filter.as_ref().is_none_or(|filter| {
                case.description.to_lowercase().contains(filter)
                    || format!("{spec_name}:{index}").to_lowercase() == *filter
            });
            let within_limit = options.max_cases.is_none_or(|limit| eligible < limit);
            if !spec_selected || !case_selected || !within_limit {
                totals.skip += 1;
                if options.verbose {
                    records.push(CaseRecord::skipped(spec_name, index, case));
                }
                continue;
            }
            eligible += 1;
            let record = if parsed.metadata.our_shell.as_deref() == Some("-") {
                CaseRecord::unsupported(
                    spec_name,
                    index,
                    case,
                    "file metadata sets our_shell to '-'",
                )
            } else {
                execute_case(
                    &context,
                    spec_name,
                    index,
                    case,
                    parsed.metadata.legacy_tmp_dir,
                )
            };
            totals.add(record.outcome);
            records.push(record);
        }
    }
    if parsed_files != group.files || parsed_cases != group.cases {
        return Err(format!(
            "group {} parsed as {parsed_files}/{parsed_cases} files/cases, manifest records {}/{}",
            group.id, group.files, group.cases
        )
        .into());
    }
    totals.executed = totals.selected - totals.skip;
    totals.assert_consistent()?;
    Ok(RunReport {
        schema: 1,
        survey: "oils-shell-spec",
        source_commit: manifest.source_commit.clone(),
        group: group.id.clone(),
        group_label: group.label.clone(),
        shell: options.reported_shell.clone(),
        shell_sha256: crate::sha256_file(&options.shell)?,
        expectation_shell: options.expectation_shell.clone(),
        containment: containment.label().to_owned(),
        posix: options.posix,
        timeout_ms: duration_millis(options.timeout),
        elapsed_ms: duration_millis(started.elapsed()),
        totals,
        cases: records,
    })
}

fn matches_spec(filter: &str, path: &str, file_name: &str) -> bool {
    let stem = file_name.strip_suffix(".test.sh").unwrap_or(file_name);
    filter == path || filter == file_name || filter == stem
}

struct RunContext<'a> {
    root: &'a Path,
    shell: &'a Path,
    expectation_shell: &'a str,
    timeout: Duration,
    posix: bool,
    survey_path: OsString,
    scratch: &'a Path,
    containment: &'a Containment,
    timezone: Option<&'a OsStr>,
    locale_archive: Option<&'a OsStr>,
}

fn execute_case(
    context: &RunContext<'_>,
    spec_name: &str,
    index: usize,
    case: &TestCase,
    legacy_tmp_dir: bool,
) -> CaseRecord {
    let directory = context
        .scratch
        .join(format!("{}-{index:04}", safe_component(spec_name)));
    let preparation = fs::create_dir(&directory).and_then(|()| {
        if legacy_tmp_dir {
            fs::create_dir(directory.join("_tmp"))?;
        }
        Ok(())
    });
    if let Err(error) = preparation {
        return CaseRecord::error(
            spec_name,
            index,
            case,
            format!("scratch directory: {error}"),
        );
    }
    match run_process(context, &directory, &case.code) {
        Ok(process) => evaluate_case(context, spec_name, index, case, process),
        Err(error) => CaseRecord::error(spec_name, index, case, format!("process: {error}")),
    }
}

fn safe_component(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn run_process(context: &RunContext<'_>, directory: &Path, code: &[u8]) -> Result<ProcessResult> {
    let mut arguments = Vec::new();
    if context.posix {
        arguments.extend([OsString::from("-o"), OsString::from("posix")]);
    }
    let mut environment = vec![
        (OsString::from("PATH"), context.survey_path.clone()),
        (OsString::from("LC_ALL"), OsString::from("C.UTF-8")),
        (
            OsString::from("LOCALE_ARCHIVE"),
            context
                .locale_archive
                .map(OsStr::to_owned)
                .unwrap_or_else(|| env::var_os("LOCALE_ARCHIVE").unwrap_or_default()),
        ),
        (OsString::from("OILS_GC_ON_EXIT"), OsString::new()),
        (
            OsString::from("REPO_ROOT"),
            context.root.as_os_str().to_owned(),
        ),
        (OsString::from("SH"), context.shell.as_os_str().to_owned()),
        (OsString::from("TMP"), directory.as_os_str().to_owned()),
    ];
    if let Some(timezone) = context.timezone {
        environment.push((OsString::from("TZ"), timezone.to_owned()));
    }
    crate::process::run(&ProcessRequest {
        containment: context.containment,
        program: context.shell,
        arguments: &arguments,
        directory,
        environment: &environment,
        input: code,
        timeout: context.timeout,
    })
}

fn evaluate_case(
    context: &RunContext<'_>,
    spec_name: &str,
    index: usize,
    case: &TestCase,
    process: ProcessResult,
) -> CaseRecord {
    let status = process
        .status
        .code()
        .unwrap_or_else(|| 128 + process.status.signal().unwrap_or(0));
    if process.timed_out {
        return CaseRecord::observed(
            spec_name,
            index,
            case,
            Outcome::Timeout,
            Some(status),
            process.duration,
            None,
            vec![],
            Some("case exceeded its deadline".to_owned()),
        );
    }
    if process.stdout.truncated || process.stderr.truncated {
        return CaseRecord::observed(
            spec_name,
            index,
            case,
            Outcome::Error,
            Some(status),
            process.duration,
            None,
            vec![],
            Some(format!(
                "captured output exceeded {OUTPUT_LIMIT} bytes per stream"
            )),
        );
    }
    if let Some(error) = process.writer_error {
        return CaseRecord::observed(
            spec_name,
            index,
            case,
            Outcome::Error,
            Some(status),
            process.duration,
            None,
            vec![],
            Some(format!("writing case input failed: {error}")),
        );
    }

    let shell_key = expectation_key(context.expectation_shell);
    let qualified = case.per_shell.get(shell_key);
    let expected_stdout = qualified
        .filter(|set| !set.assertions.stdout.is_empty())
        .map(|set| set.assertions.stdout.as_slice())
        .unwrap_or(&case.ideal.stdout);
    let expected_stderr = qualified
        .filter(|set| !set.assertions.stderr.is_empty())
        .map(|set| set.assertions.stderr.as_slice())
        .unwrap_or(&case.ideal.stderr);
    let expected_status = qualified
        .and_then(|set| set.assertions.status)
        .or(case.ideal.status)
        .unwrap_or(0);
    let qualifier_used = qualified.is_some_and(|set| {
        !set.assertions.stdout.is_empty()
            || !set.assertions.stderr.is_empty()
            || set.assertions.status.is_some()
    });
    let mut differences = Vec::new();
    for expected in expected_stdout {
        if expected.bytes != process.stdout.bytes {
            differences.push(Difference::bytes(
                &expected.source,
                &expected.bytes,
                &process.stdout.bytes,
            ));
        }
    }
    for expected in expected_stderr {
        if expected.bytes != process.stderr.bytes {
            differences.push(Difference::bytes(
                &expected.source,
                &expected.bytes,
                &process.stderr.bytes,
            ));
        }
    }
    if expected_status != status {
        differences.push(Difference::integer("status", expected_status, status));
    }
    if process
        .stderr
        .bytes
        .windows(b"Traceback (most recent".len())
        .any(|window| window == b"Traceback (most recent")
    {
        differences.push(Difference::forbidden(
            "stderr",
            b"Traceback (most recent",
            &process.stderr.bytes,
        ));
    }
    let qualifier = qualifier_used.then(|| qualified.expect("checked above").qualifier.clone());
    let outcome = if !differences.is_empty() {
        Outcome::Fail
    } else {
        match qualifier.as_deref() {
            Some("N-I") => Outcome::Unsupported,
            Some(value) if value.starts_with("BUG") => Outcome::KnownBug,
            _ => Outcome::Pass,
        }
    };
    CaseRecord::observed(
        spec_name,
        index,
        case,
        outcome,
        Some(status),
        process.duration,
        qualifier,
        differences,
        None,
    )
}

fn expectation_key(label: &str) -> &str {
    if label.starts_with("osh") {
        "osh"
    } else if label.starts_with("bash") {
        "bash"
    } else {
        label
    }
}

mod spec;
use spec::{TestCase, parse_spec};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Outcome {
    Pass,
    Fail,
    Skip,
    Unsupported,
    KnownBug,
    Timeout,
    Error,
}

#[derive(Debug, Serialize)]
struct RunReport {
    schema: u32,
    survey: &'static str,
    source_commit: String,
    group: String,
    group_label: String,
    shell: String,
    shell_sha256: String,
    expectation_shell: String,
    containment: String,
    posix: bool,
    timeout_ms: u64,
    elapsed_ms: u64,
    totals: Totals,
    cases: Vec<CaseRecord>,
}

impl RunReport {
    /// The ids of the cases that failed, deduplicated and ordered.
    ///
    /// An id is `spec:index`, exactly as the text report and the Bash
    /// gate spell it. The runner has always known these; until now the
    /// only way to get at them was to parse them back out of the text
    /// report, which is what put three underscore-named specs outside
    /// every comparison made this week.
    fn failing_ids(&self) -> BTreeSet<String> {
        self.case_ids(|outcome| outcome == Outcome::Fail)
    }

    /// The ids of the cases that decided nothing.
    ///
    /// A timeout or a harness error is not a failure and so never joins
    /// the failing set. A case that stops passing by timing out would
    /// therefore leave a list comparison perfectly silent, which is the
    /// same shape of hole the underscore was.
    fn unstable_ids(&self) -> BTreeSet<String> {
        self.case_ids(|outcome| matches!(outcome, Outcome::Timeout | Outcome::Error))
    }

    /// Every case the run reached, whatever it decided about it.
    ///
    /// A baseline entry naming none of these is stale rather than fixed,
    /// and the two must not report as the same thing.
    fn all_ids(&self) -> BTreeSet<String> {
        self.case_ids(|_| true)
    }

    fn case_ids(&self, wanted: impl Fn(Outcome) -> bool) -> BTreeSet<String> {
        self.cases
            .iter()
            .filter(|case| wanted(case.outcome))
            .map(|case| format!("{}:{}", case.spec, case.index))
            .collect()
    }

    fn write_ids(&self) {
        for id in self.failing_ids() {
            println!("{id}");
        }
    }

    fn write_text(&self, verbose: bool) {
        println!("Oils shell-spec survey: {}", self.group_label);
        println!("shell: {}", self.shell);
        println!("shell sha256: {}", self.shell_sha256);
        println!("expectations: {}", self.expectation_shell);
        println!("containment: {}", self.containment);
        println!(
            "POSIX mode: {}",
            if self.posix { "enabled" } else { "disabled" }
        );
        for case in &self.cases {
            if verbose || !matches!(case.outcome, Outcome::Pass | Outcome::Skip) {
                println!(
                    "{:11} {}:{:<4} line {:<5} {}",
                    outcome_name(case.outcome),
                    case.spec,
                    case.index,
                    case.line,
                    case.description
                );
                if let Some(note) = &case.note {
                    println!("            {note}");
                }
                for difference in &case.differences {
                    println!("            {} differs", difference.field);
                    println!("              expected: {}", difference.expected.text());
                    println!("              actual:   {}", difference.actual.text());
                }
            }
        }
        println!(
            "summary: selected={} executed={} pass={} fail={} skip={} unsupported={} known-bug={} timeout={} error={} elapsed={}ms",
            self.totals.selected,
            self.totals.executed,
            self.totals.pass,
            self.totals.fail,
            self.totals.skip,
            self.totals.unsupported,
            self.totals.known_bug,
            self.totals.timeout,
            self.totals.error,
            self.elapsed_ms
        );
    }
}

fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Pass => "PASS",
        Outcome::Fail => "FAIL",
        Outcome::Skip => "SKIP",
        Outcome::Unsupported => "UNSUPPORTED",
        Outcome::KnownBug => "KNOWN-BUG",
        Outcome::Timeout => "TIMEOUT",
        Outcome::Error => "ERROR",
    }
}

#[derive(Debug, Default, Serialize)]
struct Totals {
    selected: usize,
    executed: usize,
    pass: usize,
    fail: usize,
    skip: usize,
    unsupported: usize,
    known_bug: usize,
    timeout: usize,
    error: usize,
}

impl Totals {
    fn add(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Pass => self.pass += 1,
            Outcome::Fail => self.fail += 1,
            Outcome::Skip => self.skip += 1,
            Outcome::Unsupported => self.unsupported += 1,
            Outcome::KnownBug => self.known_bug += 1,
            Outcome::Timeout => self.timeout += 1,
            Outcome::Error => self.error += 1,
        }
    }

    fn assert_consistent(&self) -> Result<()> {
        let accounted = self.pass
            + self.fail
            + self.skip
            + self.unsupported
            + self.known_bug
            + self.timeout
            + self.error;
        if accounted != self.selected || self.executed + self.skip != self.selected {
            return Err(format!(
                "result accounting mismatch: selected={}, executed={}, accounted={accounted}",
                self.selected, self.executed
            )
            .into());
        }
        Ok(())
    }

    fn is_success(&self) -> bool {
        self.fail == 0 && self.timeout == 0 && self.error == 0
    }
}

#[derive(Serialize)]
struct ResultSummary<'a> {
    schema: u32,
    survey: &'a str,
    source_commit: &'a str,
    group: &'a str,
    group_label: &'a str,
    shell: &'a str,
    shell_sha256: &'a str,
    expectation_shell: &'a str,
    containment: &'a str,
    posix: bool,
    timeout_ms: u64,
    totals: &'a Totals,
    nonpassing: Vec<SummaryCase<'a>>,
}

#[derive(Serialize)]
struct SummaryCase<'a> {
    spec: &'a str,
    index: usize,
    line: usize,
    description: &'a str,
    outcome: Outcome,
    status: Option<i32>,
    qualifier: Option<&'a str>,
    difference_fields: Vec<&'a str>,
    note: Option<&'a str>,
}

fn write_summary(path: &Path, report: &RunReport) -> Result<()> {
    let nonpassing = report
        .cases
        .iter()
        .filter(|case| case.outcome != Outcome::Pass)
        .map(|case| SummaryCase {
            spec: &case.spec,
            index: case.index,
            line: case.line,
            description: &case.description,
            outcome: case.outcome,
            status: case.status,
            qualifier: case.qualifier.as_deref(),
            difference_fields: case
                .differences
                .iter()
                .map(|difference| difference.field.as_str())
                .collect(),
            note: case.note.as_deref(),
        })
        .collect();
    let summary = ResultSummary {
        schema: 1,
        survey: report.survey,
        source_commit: &report.source_commit,
        group: &report.group,
        group_label: &report.group_label,
        shell: &report.shell,
        shell_sha256: &report.shell_sha256,
        expectation_shell: &report.expectation_shell,
        containment: &report.containment,
        posix: report.posix,
        timeout_ms: report.timeout_ms,
        totals: &report.totals,
        nonpassing,
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(&summary)?)?;
    Ok(())
}

fn display_shell(shell: &Path) -> String {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::canonicalize(project)
        .ok()
        .and_then(|root| shell.strip_prefix(root).ok().map(Path::to_owned))
        .unwrap_or_else(|| shell.to_owned())
        .to_string_lossy()
        .replace('\\', "/")
}

#[derive(Debug, Serialize)]
struct CaseRecord {
    spec: String,
    index: usize,
    line: usize,
    description: String,
    outcome: Outcome,
    status: Option<i32>,
    duration_ms: u64,
    qualifier: Option<String>,
    differences: Vec<Difference>,
    note: Option<String>,
}

impl CaseRecord {
    fn skipped(spec: &str, index: usize, case: &TestCase) -> Self {
        Self::observed(
            spec,
            index,
            case,
            Outcome::Skip,
            None,
            Duration::ZERO,
            None,
            vec![],
            Some("excluded by the requested filters".to_owned()),
        )
    }

    fn unsupported(spec: &str, index: usize, case: &TestCase, reason: &str) -> Self {
        Self::observed(
            spec,
            index,
            case,
            Outcome::Unsupported,
            None,
            Duration::ZERO,
            None,
            vec![],
            Some(reason.to_owned()),
        )
    }

    fn error(spec: &str, index: usize, case: &TestCase, reason: String) -> Self {
        Self::observed(
            spec,
            index,
            case,
            Outcome::Error,
            None,
            Duration::ZERO,
            None,
            vec![],
            Some(reason),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn observed(
        spec: &str,
        index: usize,
        case: &TestCase,
        outcome: Outcome,
        status: Option<i32>,
        duration: Duration,
        qualifier: Option<String>,
        differences: Vec<Difference>,
        note: Option<String>,
    ) -> Self {
        Self {
            spec: spec.to_owned(),
            index,
            line: case.line,
            description: case.description.clone(),
            outcome,
            status,
            duration_ms: duration_millis(duration),
            qualifier,
            differences,
            note,
        }
    }
}

#[derive(Debug, Serialize)]
struct Difference {
    field: String,
    expected: ReportValue,
    actual: ReportValue,
}

impl Difference {
    fn bytes(field: &str, expected: &[u8], actual: &[u8]) -> Self {
        Self {
            field: field.to_owned(),
            expected: ReportValue::bytes(expected),
            actual: ReportValue::bytes(actual),
        }
    }

    fn integer(field: &str, expected: i32, actual: i32) -> Self {
        Self {
            field: field.to_owned(),
            expected: ReportValue::Integer { value: expected },
            actual: ReportValue::Integer { value: actual },
        }
    }

    fn forbidden(field: &str, forbidden: &[u8], actual: &[u8]) -> Self {
        Self {
            field: format!("{field} forbidden substring"),
            expected: ReportValue::bytes(forbidden),
            actual: ReportValue::bytes(actual),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ReportValue {
    Integer {
        value: i32,
    },
    Bytes {
        length: usize,
        sha256: String,
        utf8: Option<String>,
        hex: String,
    },
}

impl ReportValue {
    fn bytes(value: &[u8]) -> Self {
        Self::Bytes {
            length: value.len(),
            sha256: format!("{:x}", Sha256::digest(value)),
            utf8: std::str::from_utf8(value).ok().map(str::to_owned),
            hex: hex(value),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Integer { value } => value.to_string(),
            Self::Bytes {
                length,
                utf8: Some(value),
                ..
            } => format!("{value:?} ({length} bytes)"),
            Self::Bytes {
                length,
                sha256,
                hex,
                ..
            } => format!("0x{hex} ({length} bytes, sha256 {sha256})"),
        }
    }
}

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(value.len() * 2);
    for byte in value {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    result
}

fn duration_millis(duration: Duration) -> u64 {
    crate::process::duration_millis(duration)
}

#[cfg(test)]
mod tests;
