use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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

pub(crate) mod helpers;
mod reference;

pub(crate) use reference::{
    CatalogCase, GateCase, GateOutcome, ReferenceCase, ReferenceOutcome, ReferenceReport,
    ReferenceTotals, bash_case_catalog, run_gate_group, run_reference_group,
};

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

pub(crate) fn command(args: env::ArgsOs, default_root: PathBuf) -> Result<bool> {
    let Some(options) = Options::parse(args, default_root)? else {
        println!("{}", Options::usage());
        return Ok(true);
    };
    crate::read_lock(&options.root).and_then(|lock| {
        crate::verify_import(&options.root, &lock)?;
        crate::verify_oils_manifest(&options.root, &lock)
    })?;
    let manifest: crate::OilsManifest =
        toml::from_str(&fs::read_to_string(options.root.join("MANIFEST.toml"))?)?;
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
    expectation_shell: String,
    timeout: Duration,
    format: OutputFormat,
    specs: BTreeSet<String>,
    case_filter: Option<String>,
    max_cases: Option<usize>,
    summary: Option<PathBuf>,
    posix: bool,
    verbose: bool,
    base_path: Option<OsString>,
    timezone: Option<OsString>,
    locale_archive: Option<OsString>,
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
            group: "full".to_owned(),
            shell: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/nsh"),
            expectation_shell: "osh".to_owned(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            format: OutputFormat::Text,
            specs: BTreeSet::new(),
            case_filter: None,
            max_cases: None,
            summary: None,
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
                "cannot resolve shell {}: {error}; build it with cargo build --release --bin nsh",
                options.shell.display()
            )
        })?;
        Ok(Some(options))
    }

    fn usage() -> &'static str {
        "usage: nsh-survey run-oils [--group ID] [--shell PATH] [--expect-shell LABEL]\n\
                [--timeout-ms N] [--format text|json] [--spec NAME] [--case TEXT]\n\
                [--max-cases N] [--summary PATH] [--posix] [--verbose] [ROOT]"
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
    verify_shell_is_runnable(&containment, scratch.path(), &options.shell)?;
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
        shell: display_shell(&options.shell),
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

#[derive(Debug)]
struct ParsedFile {
    metadata: FileMetadata,
    cases: Vec<TestCase>,
}

#[derive(Debug, Default)]
struct FileMetadata {
    values: BTreeMap<String, String>,
    our_shell: Option<String>,
    legacy_tmp_dir: bool,
}

#[derive(Debug)]
struct TestCase {
    description: String,
    line: usize,
    code: Vec<u8>,
    ideal: Assertions,
    per_shell: BTreeMap<String, QualifiedAssertions>,
}

#[derive(Clone, Debug, Default)]
struct Assertions {
    stdout: Vec<ExpectedBytes>,
    stderr: Vec<ExpectedBytes>,
    status: Option<i32>,
}

#[derive(Clone, Debug)]
struct ExpectedBytes {
    source: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct QualifiedAssertions {
    qualifier: String,
    assertions: Assertions,
}

#[derive(Debug)]
struct CaseBuilder {
    description: String,
    line: usize,
    code: Option<Vec<u8>>,
    ideal: Assertions,
    per_shell: BTreeMap<String, QualifiedAssertions>,
}

fn parse_spec(path: &Path) -> Result<ParsedFile> {
    let bytes = fs::read(path)?;
    parse_spec_bytes(&bytes).map_err(|error| format!("{}: {error}", path.display()).into())
}

fn parse_spec_bytes(bytes: &[u8]) -> Result<ParsedFile> {
    let mut tokens = Tokenizer::new(bytes)?;
    let mut metadata = FileMetadata::default();
    while let Token::Key(key) = tokens.peek().token.clone() {
        if key.qualifier.is_some() {
            return Err(format!("line {}: qualifier in file metadata", key.line).into());
        }
        let value = String::from_utf8(key.value)?;
        if metadata.values.insert(key.name.clone(), value).is_some() {
            return Err(format!("line {}: duplicate file metadata {}", key.line, key.name).into());
        }
        tokens.advance(LexMode::Outer)?;
    }
    const FILE_FIELDS: &[&str] = &[
        "our_shell",
        "compare_shells",
        "suite",
        "tags",
        "oils_failures_allowed",
        "oils_cpp_failures_allowed",
        "legacy_tmp_dir",
    ];
    if let Some(invalid) = metadata
        .values
        .keys()
        .find(|name| !FILE_FIELDS.contains(&name.as_str()))
    {
        return Err(format!("invalid file metadata {invalid:?}").into());
    }
    metadata.our_shell = metadata.values.get("our_shell").cloned();
    metadata.legacy_tmp_dir = metadata
        .values
        .get("legacy_tmp_dir")
        .is_some_and(|value| !value.is_empty());

    let mut cases = Vec::new();
    while !matches!(tokens.peek().token, Token::Eof) {
        cases.push(parse_case(&mut tokens)?);
    }
    Ok(ParsedFile { metadata, cases })
}

fn parse_case(tokens: &mut Tokenizer<'_>) -> Result<TestCase> {
    let (description, line) = match &tokens.peek().token {
        Token::CaseBegin(description) => (description.clone(), tokens.peek().line),
        token => {
            return Err(format!(
                "line {}: expected case heading, got {token:?}",
                tokens.peek().line
            )
            .into());
        }
    };
    tokens.advance(LexMode::Outer)?;
    let mut builder = CaseBuilder {
        description,
        line,
        code: None,
        ideal: Assertions::default(),
        per_shell: BTreeMap::new(),
    };
    parse_case_metadata(tokens, &mut builder)?;
    if builder.code.is_none() {
        let mut code = Vec::new();
        if !matches!(tokens.peek().token, Token::Plain(_)) {
            return Err(format!("line {}: expected case code", tokens.peek().line).into());
        }
        while let Token::Plain(line) = &tokens.peek().token {
            code.extend_from_slice(line);
            tokens.advance(LexMode::Raw)?;
        }
        builder.code = Some(code);
        parse_case_metadata(tokens, &mut builder)?;
    }
    Ok(TestCase {
        description: builder.description,
        line: builder.line,
        code: builder.code.expect("case code assigned"),
        ideal: builder.ideal,
        per_shell: builder.per_shell,
    })
}

fn parse_case_metadata(tokens: &mut Tokenizer<'_>, builder: &mut CaseBuilder) -> Result<()> {
    loop {
        match tokens.peek().token.clone() {
            Token::Key(key) => {
                apply_case_metadata(builder, key)?;
                tokens.advance(LexMode::Outer)?;
            }
            Token::Multiline(mut key) => {
                if !key.value.is_empty() {
                    return Err(format!(
                        "line {}: multiline {} value must start on the following line",
                        key.line, key.name
                    )
                    .into());
                }
                tokens.advance(LexMode::Raw)?;
                let mut value = Vec::new();
                while let Token::Plain(line) = &tokens.peek().token {
                    value.extend_from_slice(line);
                    tokens.advance(LexMode::Raw)?;
                }
                if matches!(tokens.peek().token, Token::End) {
                    tokens.advance(LexMode::Outer)?;
                }
                key.name.make_ascii_lowercase();
                key.value = value;
                apply_case_metadata(builder, key)?;
            }
            _ => return Ok(()),
        }
    }
}

fn apply_case_metadata(builder: &mut CaseBuilder, key: KeyValue) -> Result<()> {
    if key.name == "code" {
        if key.qualifier.is_some() {
            return Err(format!("line {}: code cannot be shell-qualified", key.line).into());
        }
        if builder.code.replace(key.value).is_some() {
            return Err(format!("line {}: duplicate code", key.line).into());
        }
        return Ok(());
    }
    if let Some(qualifier) = key.qualifier {
        for shell in key.shells {
            let qualified =
                builder
                    .per_shell
                    .entry(shell.clone())
                    .or_insert_with(|| QualifiedAssertions {
                        qualifier: qualifier.clone(),
                        assertions: Assertions::default(),
                    });
            if qualified.qualifier != qualifier {
                return Err(format!(
                    "line {}: inconsistent qualifier for {shell}: {} versus {qualifier}",
                    key.line, qualified.qualifier
                )
                .into());
            }
            set_assertion(
                &mut qualified.assertions,
                &key.name,
                &key.value,
                key.line,
                true,
            )?;
        }
    } else {
        set_assertion(&mut builder.ideal, &key.name, &key.value, key.line, false)?;
    }
    Ok(())
}

fn set_assertion(
    set: &mut Assertions,
    name: &str,
    value: &[u8],
    line: usize,
    reject_duplicate_base: bool,
) -> Result<()> {
    match name {
        "stdout" => set_bytes(
            &mut set.stdout,
            name,
            value.to_vec(),
            line,
            reject_duplicate_base,
        ),
        "stderr" => set_bytes(
            &mut set.stderr,
            name,
            value.to_vec(),
            line,
            reject_duplicate_base,
        ),
        "stdout-json" => {
            let decoded: String = serde_json::from_str(std::str::from_utf8(value)?)?;
            set_bytes(
                &mut set.stdout,
                name,
                decoded.into_bytes(),
                line,
                reject_duplicate_base,
            )
        }
        "stderr-json" => {
            let decoded: String = serde_json::from_str(std::str::from_utf8(value)?)?;
            set_bytes(
                &mut set.stderr,
                name,
                decoded.into_bytes(),
                line,
                reject_duplicate_base,
            )
        }
        "status" => {
            if reject_duplicate_base && set.status.is_some() {
                return Err(format!("line {line}: duplicate status assertion").into());
            }
            set.status = Some(std::str::from_utf8(value)?.trim().parse()?);
            Ok(())
        }
        // A small number of upstream files spell the optional multiline
        // terminator as `## END:`. Oils tokenizes that as inert case metadata.
        "END" => Ok(()),
        _ => Err(format!("line {line}: unsupported case metadata {name:?}").into()),
    }
}

fn set_bytes(
    slot: &mut Vec<ExpectedBytes>,
    source: &str,
    value: Vec<u8>,
    line: usize,
    reject_duplicate_base: bool,
) -> Result<()> {
    if reject_duplicate_base && !slot.is_empty() {
        let base = source.strip_suffix("-json").unwrap_or(source);
        return Err(format!("line {line}: duplicate {base} assertion").into());
    }
    if let Some(existing) = slot.iter_mut().find(|expected| expected.source == source) {
        existing.bytes = value;
    } else {
        slot.push(ExpectedBytes {
            source: source.to_owned(),
            bytes: value,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum LexMode {
    Outer,
    Raw,
}

#[derive(Clone, Debug)]
struct SpannedToken {
    line: usize,
    token: Token,
}

#[derive(Clone, Debug)]
enum Token {
    CaseBegin(String),
    Key(KeyValue),
    Multiline(KeyValue),
    End,
    Plain(Vec<u8>),
    Eof,
}

#[derive(Clone, Debug)]
struct KeyValue {
    line: usize,
    qualifier: Option<String>,
    shells: Vec<String>,
    name: String,
    value: Vec<u8>,
}

struct Tokenizer<'a> {
    lines: Vec<&'a [u8]>,
    next: usize,
    cursor: SpannedToken,
}

impl<'a> Tokenizer<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let mut tokenizer = Self {
            lines: bytes.split_inclusive(|byte| *byte == b'\n').collect(),
            next: 0,
            cursor: SpannedToken {
                line: 0,
                token: Token::Eof,
            },
        };
        tokenizer.advance(LexMode::Outer)?;
        Ok(tokenizer)
    }

    fn peek(&self) -> &SpannedToken {
        &self.cursor
    }

    fn advance(&mut self, mode: LexMode) -> Result<()> {
        loop {
            if self.next == self.lines.len() {
                self.cursor = SpannedToken {
                    line: self.next + 1,
                    token: Token::Eof,
                };
                return Ok(());
            }
            let line_number = self.next + 1;
            let line = self.lines[self.next];
            self.next += 1;
            if let Some(token) = classify_line(line, line_number, mode)? {
                self.cursor = SpannedToken {
                    line: line_number,
                    token,
                };
                return Ok(());
            }
        }
    }
}

fn classify_line(line: &[u8], line_number: usize, mode: LexMode) -> Result<Option<Token>> {
    if matches!(mode, LexMode::Outer) && trim_ascii(line).is_empty() {
        return Ok(None);
    }
    if let Some(rest) = line.strip_prefix(b"####") {
        return Ok(Some(Token::CaseBegin(String::from_utf8(
            trim_ascii(rest).to_vec(),
        )?)));
    }
    if let Some(key) = parse_key_value(line, line_number)? {
        return Ok(Some(if matches!(key.name.as_str(), "STDOUT" | "STDERR") {
            Token::Multiline(key)
        } else {
            Token::Key(key)
        }));
    }
    if is_end_marker(line) {
        return Ok(Some(Token::End));
    }
    if line.starts_with(b"##") {
        return Err(format!("line {line_number}: invalid ## metadata line").into());
    }
    if trim_start_ascii(line).starts_with(b"#") {
        return Ok(None);
    }
    Ok(Some(Token::Plain(line.to_vec())))
}

fn parse_key_value(line: &[u8], line_number: usize) -> Result<Option<KeyValue>> {
    let Some(after_hashes) = line.strip_prefix(b"##") else {
        return Ok(None);
    };
    if !after_hashes.first().is_some_and(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let content = strip_line_ending(trim_start_ascii(after_hashes));
    let Some(colon) = content.iter().position(|byte| *byte == b':') else {
        return Ok(None);
    };
    let words: Vec<&[u8]> = content[..colon]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .collect();
    let (qualifier, shells, name) = match words.as_slice() {
        [name] if valid_key(name) => (None, Vec::new(), *name),
        [qualifier, shells, name]
            if valid_qualifier(qualifier) && valid_shells(shells) && valid_key(name) =>
        {
            (
                Some(String::from_utf8(qualifier.to_vec())?),
                String::from_utf8(shells.to_vec())?
                    .split('/')
                    .map(str::to_owned)
                    .collect(),
                *name,
            )
        }
        _ => return Ok(None),
    };
    let mut value = trim_start_ascii(&content[colon + 1..]).to_vec();
    let name = String::from_utf8(name.to_vec())?;
    if matches!(name.as_str(), "stdout" | "stderr") {
        value.push(b'\n');
    }
    Ok(Some(KeyValue {
        line: line_number,
        qualifier,
        shells,
        name,
        value,
    }))
}

fn valid_key(value: &[u8]) -> bool {
    !value.is_empty()
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_shells(value: &[u8]) -> bool {
    !value.is_empty()
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'/'))
        && !value.starts_with(b"/")
        && !value.ends_with(b"/")
        && !value.windows(2).any(|window| window == b"//")
}

fn valid_qualifier(value: &[u8]) -> bool {
    value == b"OK"
        || value == b"BUG"
        || value == b"N-I"
        || value
            .strip_prefix(b"OK-")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit))
        || value
            .strip_prefix(b"BUG-")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit))
}

fn is_end_marker(line: &[u8]) -> bool {
    line.strip_prefix(b"##")
        .filter(|rest| rest.first().is_some_and(u8::is_ascii_whitespace))
        .map(trim_start_ascii)
        .is_some_and(|rest| rest.starts_with(b"END"))
}

fn strip_line_ending(mut line: &[u8]) -> &[u8] {
    if let Some(without) = line.strip_suffix(b"\n") {
        line = without;
    }
    if let Some(without) = line.strip_suffix(b"\r") {
        line = without;
    }
    line
}

fn trim_start_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    &value[start..]
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let value = trim_start_ascii(value);
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |index| index + 1);
    &value[..end]
}

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

/// Refuse to score a shell the containment cannot reach.
///
/// Every case runs inside a sandbox that replaces `/tmp` with an empty
/// tmpfs, so a shell living under `/tmp` is simply not there once the
/// boundary is up. Each case then fails to start it, and the run reports
/// one ordinary failure per case -- a number that looks exactly like a
/// real measurement of a very bad shell, and is not a measurement at all.
///
/// This asks the question once, inside the same boundary the cases use,
/// and turns "I could not run it" into an error instead of a score. It
/// deliberately tests only that the file is there and executable: a shell
/// that starts and then behaves badly must still be *measured* badly
/// rather than refused, which is what keeps a stub distinguishable from
/// Bash instead of both being errors.
// [spec:nsh:req:compat.bash.survey-closure]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
fn verify_shell_is_runnable(
    containment: &Containment,
    directory: &Path,
    shell: &Path,
) -> Result<()> {
    let arguments = [
        OsString::from("-c"),
        OsString::from("test -x \"$1\""),
        OsString::from("nsh-survey-shell-probe"),
        shell.as_os_str().to_owned(),
    ];
    let output = crate::process::run(&crate::process::Request {
        containment,
        program: Path::new("/bin/sh"),
        arguments: &arguments,
        directory,
        environment: &[],
        input: b"",
        timeout: Duration::from_secs(5),
    })?;
    if !output.status.success() {
        return Err(format!(
            "the shell under test is not executable inside the survey containment: {}. \
             The boundary mounts an empty tmpfs over /tmp, so a shell kept there is \
             invisible to every case; give a path outside /tmp.",
            shell.display()
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
