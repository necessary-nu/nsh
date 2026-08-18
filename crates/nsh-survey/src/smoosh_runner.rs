use serde::Serialize;
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::process::{Output as ProcessOutput, Request as ProcessRequest, ScratchTree};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) mod helpers;

pub(crate) fn command(args: env::ArgsOs, default_root: PathBuf) -> Result<bool> {
    let Some(options) = Options::parse(args, default_root)? else {
        println!("{}", Options::usage());
        return Ok(true);
    };
    let lock = crate::smoosh::read_lock(&options.root)?;
    let manifest = crate::smoosh::read_manifest(&options.root, &lock)?;
    let report = run_manifest(&options, &manifest)?;
    if let Some(path) = &options.summary {
        write_summary(path, &report)?;
    }
    match options.format {
        OutputFormat::Text => report.write_text(options.verbose),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(report.totals.is_success())
}

#[derive(Debug)]
struct Options {
    root: PathBuf,
    group: String,
    shell: PathBuf,
    shell_flags: Vec<String>,
    default_timeout: Option<Duration>,
    known_hang_timeout: Option<Duration>,
    format: OutputFormat,
    tests: BTreeSet<String>,
    max_tests: Option<usize>,
    summary: Option<PathBuf>,
    verbose: bool,
}

#[derive(Clone, Copy, Debug)]
enum OutputFormat {
    Text,
    Json,
}

impl Options {
    fn parse(mut args: env::ArgsOs, default_root: PathBuf) -> Result<Option<Self>> {
        let mut options = Self {
            root: default_root,
            group: "regular".to_owned(),
            shell: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/nsh"),
            shell_flags: Vec::new(),
            default_timeout: None,
            known_hang_timeout: None,
            format: OutputFormat::Text,
            tests: BTreeSet::new(),
            max_tests: None,
            summary: None,
            verbose: false,
        };
        let mut root_seen = false;
        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("-h" | "--help") => return Ok(None),
                Some("--group") => options.group = required_string(&mut args, "--group")?,
                Some("--shell") => options.shell = required_path(&mut args, "--shell")?,
                Some("--shell-flag") => options
                    .shell_flags
                    .push(required_string(&mut args, "--shell-flag")?),
                Some("--timeout-ms") => {
                    options.default_timeout = Some(required_timeout(&mut args, "--timeout-ms")?)
                }
                Some("--known-hang-timeout-ms") => {
                    options.known_hang_timeout =
                        Some(required_timeout(&mut args, "--known-hang-timeout-ms")?)
                }
                Some("--format") => {
                    options.format = match required_string(&mut args, "--format")?.as_str() {
                        "text" => OutputFormat::Text,
                        "json" => OutputFormat::Json,
                        value => return Err(format!("unsupported output format {value:?}").into()),
                    }
                }
                Some("--test") => {
                    options.tests.insert(required_string(&mut args, "--test")?);
                }
                Some("--max-tests") => {
                    options.max_tests = Some(required_string(&mut args, "--max-tests")?.parse()?)
                }
                Some("--summary") => options.summary = Some(required_path(&mut args, "--summary")?),
                Some("--verbose") => options.verbose = true,
                Some(value) if value.starts_with('-') => {
                    return Err(
                        format!("unknown run-smoosh option {value:?}; {}", Self::usage()).into(),
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
                "cannot resolve Smoosh survey root {}: {error}",
                options.root.display()
            )
        })?;
        options.shell = fs::canonicalize(&options.shell).map_err(|error| {
            format!(
                "cannot resolve shell {}: {error}; build it with cargo build --release --bin nsh",
                options.shell.display()
            )
        })?;
        Ok(Some(options))
    }

    fn usage() -> &'static str {
        "usage: nsh-survey run-smoosh [--group regular|known-hang|full] [--shell PATH]\n\
                [--shell-flag FLAG] [--timeout-ms N] [--known-hang-timeout-ms N]\n\
                [--format text|json] [--test NAME] [--max-tests N] [--summary PATH]\n\
                [--verbose] [ROOT]"
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

fn required_timeout(args: &mut env::ArgsOs, option: &str) -> Result<Duration> {
    let milliseconds: u64 = required_string(args, option)?.parse()?;
    if milliseconds == 0 || milliseconds > 600_000 {
        return Err(format!("{option} must be between 1 and 600000").into());
    }
    Ok(Duration::from_millis(milliseconds))
}

fn run_manifest(options: &Options, manifest: &crate::smoosh::Manifest) -> Result<RunReport> {
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
                "unknown Smoosh group {:?}; choose one of {choices}",
                options.group
            )
        })?;
    let default_timeout = options
        .default_timeout
        .unwrap_or_else(|| Duration::from_millis(manifest.timeouts.default_ms));
    let known_hang_timeout = options
        .known_hang_timeout
        .unwrap_or_else(|| Duration::from_millis(manifest.timeouts.known_hang_ms));
    let scratch = ScratchTree::new()?;
    let fixture = Fixture::install(scratch.path())?;
    let flags_json = serde_json::to_string(&options.shell_flags)?;
    let inherited_path = env::var_os("PATH").unwrap_or_default();
    let logname = env::var_os("LOGNAME")
        .or_else(|| env::var_os("USER"))
        .unwrap_or_else(|| OsString::from("nsh-survey"));
    let shell_flags = options.shell_flags.join(" ");
    let common_environment = vec![
        (OsString::from("PATH"), inherited_path),
        (OsString::from("LC_ALL"), OsString::from("C.UTF-8")),
        (
            OsString::from("LOCALE_ARCHIVE"),
            env::var_os("LOCALE_ARCHIVE").unwrap_or_default(),
        ),
        (OsString::from("HOME"), fixture.home.as_os_str().to_owned()),
        (OsString::from("LOGNAME"), logname),
        (
            OsString::from("TEST_SHELL"),
            fixture.shell.as_os_str().to_owned(),
        ),
        (
            OsString::from("TEST_SHELL_FLAGS"),
            OsString::from(shell_flags),
        ),
        (
            OsString::from("TEST_UTIL"),
            fixture.util.as_os_str().to_owned(),
        ),
        (
            OsString::from("NSH_SURVEY_SMOOSH_SHELL"),
            options.shell.as_os_str().to_owned(),
        ),
        (
            OsString::from("NSH_SURVEY_SMOOSH_FLAGS_JSON"),
            OsString::from(flags_json),
        ),
    ];

    let started = Instant::now();
    let mut totals = Totals {
        selected: group.tests,
        ..Totals::default()
    };
    let mut records = Vec::new();
    let mut eligible = 0_usize;
    for test in manifest
        .tests
        .iter()
        .filter(|test| test.groups.iter().any(|candidate| candidate == &group.id))
    {
        let selected = options.tests.is_empty()
            || options
                .tests
                .iter()
                .any(|filter| matches_test(filter, &test.name));
        let within_limit = options.max_tests.is_none_or(|limit| eligible < limit);
        if !selected || !within_limit {
            totals.skip += 1;
            continue;
        }
        eligible += 1;
        let timeout = if test.known_hang {
            known_hang_timeout
        } else {
            default_timeout
        };
        let record = execute_test(&fixture, &common_environment, &options.root, test, timeout);
        totals.add(record.outcome);
        records.push(record);
    }
    totals.executed = eligible;
    totals.assert_consistent()?;
    Ok(RunReport {
        schema: 1,
        survey: manifest.survey.clone(),
        source_commit: manifest.source_commit.clone(),
        group: group.id.clone(),
        group_label: group.label.clone(),
        shell: display_shell(&options.shell),
        shell_sha256: crate::sha256_file(&options.shell)?,
        shell_flags: options.shell_flags.clone(),
        posix_mode: if options.shell_flags.is_empty() {
            "shell-native".to_owned()
        } else {
            "explicit-flags-applied-to-all-invocations".to_owned()
        },
        default_timeout_ms: crate::process::duration_millis(default_timeout),
        known_hang_timeout_ms: crate::process::duration_millis(known_hang_timeout),
        elapsed_ms: crate::process::duration_millis(started.elapsed()),
        totals,
        cases: records,
    })
}

fn matches_test(filter: &str, name: &str) -> bool {
    filter == name || name.strip_suffix(".test") == Some(filter)
}

struct Fixture {
    shell: PathBuf,
    util: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn install(scratch: &Path) -> Result<Self> {
        let root = scratch.join("smoosh-fixture");
        let util = root.join("util");
        let home = root.join("home");
        fs::create_dir_all(&util)?;
        fs::create_dir(&home)?;
        let executable = env::current_exe()?;
        let shell = root.join("smoosh-shell");
        symlink(&executable, &shell)?;
        for name in ["argv", "fds", "getenv", "readdir"] {
            symlink(&executable, util.join(name))?;
        }
        Ok(Self { shell, util, home })
    }
}

fn execute_test(
    fixture: &Fixture,
    common_environment: &[(OsString, OsString)],
    root: &Path,
    test: &crate::smoosh::ManifestTest,
    timeout: Duration,
) -> CaseRecord {
    let directory = fixture
        .home
        .parent()
        .expect("fixture home has a parent")
        .parent()
        .expect("fixture root has a parent")
        .join("cases")
        .join(safe_component(&test.name));
    if let Err(error) = fs::create_dir_all(&directory) {
        return CaseRecord::error(test, timeout, format!("scratch directory: {error}"));
    }
    let mut environment = common_environment.to_vec();
    environment.push((OsString::from("TMP"), directory.as_os_str().to_owned()));
    let arguments = [root.join(&test.script.path).into_os_string()];
    let process = crate::process::run(&ProcessRequest {
        program: &fixture.shell,
        arguments: &arguments,
        directory: &directory,
        environment: &environment,
        input: b"",
        timeout,
    });
    match process {
        Ok(process) => evaluate_test(root, test, timeout, process),
        Err(error) => CaseRecord::error(test, timeout, format!("process: {error}")),
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

fn evaluate_test(
    root: &Path,
    test: &crate::smoosh::ManifestTest,
    timeout: Duration,
    process: ProcessOutput,
) -> CaseRecord {
    let status = process
        .status
        .code()
        .unwrap_or_else(|| 128 + process.status.signal().unwrap_or(0));
    if process.timed_out {
        return CaseRecord::observed(
            test,
            timeout,
            Outcome::Timeout,
            Some(status),
            process.duration,
            Vec::new(),
            Some("test exceeded its deadline".to_owned()),
        );
    }
    if process.stdout.truncated || process.stderr.truncated {
        return CaseRecord::observed(
            test,
            timeout,
            Outcome::Error,
            Some(status),
            process.duration,
            Vec::new(),
            Some(format!(
                "captured output exceeded {} bytes per stream",
                crate::process::OUTPUT_LIMIT
            )),
        );
    }
    if let Some(error) = process.writer_error {
        return CaseRecord::observed(
            test,
            timeout,
            Outcome::Error,
            Some(status),
            process.duration,
            Vec::new(),
            Some(format!("closing empty test input failed: {error}")),
        );
    }
    let mut differences = Vec::new();
    if let Err(error) = compare_optional(
        root,
        "stdout",
        test.stdout.as_ref(),
        &process.stdout.bytes,
        &mut differences,
    ) {
        return CaseRecord::error(test, timeout, error.to_string());
    }
    if let Err(error) = compare_optional(
        root,
        "stderr",
        test.stderr.as_ref(),
        &process.stderr.bytes,
        &mut differences,
    ) {
        return CaseRecord::error(test, timeout, error.to_string());
    }
    if status != test.expected_status {
        differences.push(Difference::Status {
            expected: test.expected_status,
            actual: status,
        });
    }
    let outcome = if differences.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail
    };
    CaseRecord::observed(
        test,
        timeout,
        outcome,
        Some(status),
        process.duration,
        differences,
        None,
    )
}

fn compare_optional(
    root: &Path,
    field: &str,
    oracle: Option<&crate::smoosh::OracleFile>,
    actual: &[u8],
    differences: &mut Vec<Difference>,
) -> Result<()> {
    let Some(oracle) = oracle else {
        return Ok(());
    };
    let expected = fs::read(root.join(&oracle.path))?;
    if expected != actual {
        differences.push(Difference::Bytes {
            field: field.to_owned(),
            expected_len: expected.len(),
            actual_len: actual.len(),
            expected_hex: hex(&expected),
            actual_hex: hex(actual),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Outcome {
    Pass,
    Fail,
    Timeout,
    Error,
}

#[derive(Debug, Serialize)]
struct RunReport {
    schema: u32,
    survey: String,
    source_commit: String,
    group: String,
    group_label: String,
    shell: String,
    shell_sha256: String,
    shell_flags: Vec<String>,
    posix_mode: String,
    default_timeout_ms: u64,
    known_hang_timeout_ms: u64,
    elapsed_ms: u64,
    totals: Totals,
    cases: Vec<CaseRecord>,
}

impl RunReport {
    fn write_text(&self, verbose: bool) {
        println!("Smoosh POSIX survey: {}", self.group);
        println!("source: {}", self.source_commit);
        println!("shell: {}", self.shell);
        println!("POSIX mode: {}", self.posix_mode);
        for case in &self.cases {
            if verbose || case.outcome != Outcome::Pass {
                println!(
                    "{} {:<7} {} ({} ms)",
                    if case.known_hang { "HANG" } else { "TEST" },
                    outcome_name(case.outcome),
                    case.name,
                    case.duration_ms
                );
                for difference in &case.differences {
                    println!("  {}", difference.describe());
                }
                if let Some(error) = &case.error {
                    println!("  {error}");
                }
            }
        }
        println!(
            "selected={} executed={} pass={} fail={} timeout={} error={} skip={} elapsed={}ms",
            self.totals.selected,
            self.totals.executed,
            self.totals.pass,
            self.totals.fail,
            self.totals.timeout,
            self.totals.error,
            self.totals.skip,
            self.elapsed_ms
        );
    }
}

fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Pass => "PASS",
        Outcome::Fail => "FAIL",
        Outcome::Timeout => "TIMEOUT",
        Outcome::Error => "ERROR",
    }
}

#[derive(Clone, Debug, Default, Serialize)]
struct Totals {
    selected: usize,
    executed: usize,
    pass: usize,
    fail: usize,
    timeout: usize,
    error: usize,
    skip: usize,
}

impl Totals {
    fn add(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Pass => self.pass += 1,
            Outcome::Fail => self.fail += 1,
            Outcome::Timeout => self.timeout += 1,
            Outcome::Error => self.error += 1,
        }
    }

    fn assert_consistent(&self) -> Result<()> {
        if self.executed != self.pass + self.fail + self.timeout + self.error
            || self.selected != self.executed + self.skip
        {
            return Err(format!("inconsistent Smoosh totals: {self:?}").into());
        }
        Ok(())
    }

    fn is_success(&self) -> bool {
        self.fail == 0 && self.timeout == 0 && self.error == 0
    }
}

#[derive(Debug, Serialize)]
struct CaseRecord {
    name: String,
    outcome: Outcome,
    known_hang: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    known_hang_reason: Option<String>,
    timeout_ms: u64,
    status: Option<i32>,
    duration_ms: u64,
    differences: Vec<Difference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl CaseRecord {
    fn observed(
        test: &crate::smoosh::ManifestTest,
        timeout: Duration,
        outcome: Outcome,
        status: Option<i32>,
        duration: Duration,
        differences: Vec<Difference>,
        error: Option<String>,
    ) -> Self {
        Self {
            name: test.name.clone(),
            outcome,
            known_hang: test.known_hang,
            known_hang_reason: test.known_hang_reason.clone(),
            timeout_ms: crate::process::duration_millis(timeout),
            status,
            duration_ms: crate::process::duration_millis(duration),
            differences,
            error,
        }
    }

    fn error(test: &crate::smoosh::ManifestTest, timeout: Duration, error: String) -> Self {
        Self::observed(
            test,
            timeout,
            Outcome::Error,
            None,
            Duration::ZERO,
            Vec::new(),
            Some(error),
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Difference {
    Bytes {
        field: String,
        expected_len: usize,
        actual_len: usize,
        expected_hex: String,
        actual_hex: String,
    },
    Status {
        expected: i32,
        actual: i32,
    },
}

impl Difference {
    fn describe(&self) -> String {
        match self {
            Self::Bytes {
                field,
                expected_len,
                actual_len,
                ..
            } => format!(
                "{field} differs: expected {expected_len} byte(s), got {actual_len} byte(s)"
            ),
            Self::Status { expected, actual } => {
                format!("status differs: expected {expected}, got {actual}")
            }
        }
    }
}

#[derive(Serialize)]
struct ResultSummary<'a> {
    schema: u32,
    survey: &'a str,
    source_commit: &'a str,
    group: &'a str,
    shell: &'a str,
    shell_sha256: &'a str,
    shell_flags: &'a [String],
    posix_mode: &'a str,
    default_timeout_ms: u64,
    known_hang_timeout_ms: u64,
    totals: &'a Totals,
    nonpassing: Vec<SummaryCase<'a>>,
}

#[derive(Serialize)]
struct SummaryCase<'a> {
    name: &'a str,
    outcome: Outcome,
    known_hang: bool,
}

fn write_summary(path: &Path, report: &RunReport) -> Result<()> {
    let nonpassing = report
        .cases
        .iter()
        .filter(|case| case.outcome != Outcome::Pass)
        .map(|case| SummaryCase {
            name: &case.name,
            outcome: case.outcome,
            known_hang: case.known_hang,
        })
        .collect();
    let summary = ResultSummary {
        schema: 1,
        survey: &report.survey,
        source_commit: &report.source_commit,
        group: &report.group,
        shell: &report.shell,
        shell_sha256: &report.shell_sha256,
        shell_flags: &report.shell_flags,
        posix_mode: &report.posix_mode,
        default_timeout_ms: report.default_timeout_ms,
        known_hang_timeout_ms: report.known_hang_timeout_ms,
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

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(value.len() * 2);
    for byte in value {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_groups_are_distinct() {
        let root = crate::smoosh::survey_root();
        let lock = crate::smoosh::read_lock(&root).unwrap();
        let manifest = crate::smoosh::read_manifest(&root, &lock).unwrap();
        assert_eq!(manifest.groups[0].tests, 186);
        assert_eq!(manifest.groups[1].tests, 179);
        assert_eq!(manifest.groups[2].tests, 7);
        assert_eq!(
            manifest.tests.iter().filter(|test| test.known_hang).count(),
            7
        );
    }

    #[test]
    fn test_filter_accepts_suffixless_name() {
        assert!(matches_test(
            "semantics.tilde.sep",
            "semantics.tilde.sep.test"
        ));
        assert!(matches_test(
            "semantics.tilde.sep.test",
            "semantics.tilde.sep.test"
        ));
        assert!(!matches_test("tilde", "semantics.tilde.sep.test"));
    }

    #[test]
    fn byte_differences_keep_exact_hex() {
        assert_eq!(hex(b"\0\xfftext"), "00ff74657874");
        let difference = Difference::Bytes {
            field: "stdout".to_owned(),
            expected_len: 2,
            actual_len: 4,
            expected_hex: "00ff".to_owned(),
            actual_hex: "74657874".to_owned(),
        };
        assert_eq!(
            difference.describe(),
            "stdout differs: expected 2 byte(s), got 4 byte(s)"
        );
    }
}
