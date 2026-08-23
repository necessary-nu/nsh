use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

mod bash_gate;
mod bash_reference;
mod oils_runner;
mod process;
mod smoosh;
mod smoosh_runner;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, Deserialize)]
struct SourceLock {
    schema: u32,
    repository: String,
    commit: String,
    tree: String,
    license: String,
    license_path: String,
    license_sha256: String,
    spec_format: String,
    observed: Observed,
    manifests: Option<ManifestExpectations>,
}

#[derive(Debug, Deserialize)]
struct Observed {
    all_files: usize,
    all_cases: usize,
    active_osh_files: usize,
    active_osh_cases: usize,
}

#[derive(Debug, Deserialize)]
struct ManifestExpectations {
    full: ExpectedCount,
    posix_candidate: ExpectedCount,
    bash_comparison: ExpectedCount,
    bash_extension: ExpectedCount,
    bash_named_diagnostic: ExpectedCount,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
struct ExpectedCount {
    files: usize,
    cases: usize,
}

fn main() {
    if let Some(status) = oils_runner::helpers::status_if_invoked() {
        std::process::exit(status);
    }
    if let Some(status) = smoosh_runner::helpers::status_if_invoked() {
        std::process::exit(status);
    }
    if let Err(error) = run() {
        eprintln!("nsh-survey: {error}");
        std::process::exit(1);
    }
}

// The survey entrypoint verifies pinned inputs before dispatching every
// compatibility run; the repository-level wrapper supplies the outer,
// terminal-safe containment boundary.
// [spec:nsh:req:idiom.conformance-closure]
fn run() -> Result<()> {
    let mut args = env::args_os();
    let _program = args.next();
    match args.next().as_deref() {
        Some(command) if command == OsStr::new("import-oils") => {
            let checkout = required_path(args.next(), "OILS_CHECKOUT")?;
            let output = args.next().map(PathBuf::from).unwrap_or_else(survey_root);
            reject_extra_args(args)?;
            import_oils(&checkout, &output, &survey_root())
        }
        Some(command) if command == OsStr::new("verify-oils") => {
            let root = args.next().map(PathBuf::from).unwrap_or_else(survey_root);
            reject_extra_args(args)?;
            verify_oils(&root)
        }
        Some(command) if command == OsStr::new("generate-oils-manifests") => {
            let root = args.next().map(PathBuf::from).unwrap_or_else(survey_root);
            reject_extra_args(args)?;
            generate_oils_manifests(&root)
        }
        Some(command) if command == OsStr::new("run-oils") => {
            if oils_runner::command(args, survey_root())? {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }
        Some(command) if command == OsStr::new("gate-bash") => {
            if bash_gate::command(args, survey_root())? {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }
        Some(command) if command == OsStr::new("build-bash-reference") => {
            bash_reference::build_command(args)
        }
        Some(command) if command == OsStr::new("calibrate-bash-reference") => {
            bash_reference::calibrate_command(args, survey_root())
        }
        Some(command) if command == OsStr::new("verify-bash-reference") => {
            bash_reference::verify_command(args, survey_root())
        }
        Some(command) if command == OsStr::new("import-smoosh") => {
            let checkout = required_path(args.next(), "SMOOSH_CHECKOUT")?;
            let output = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(smoosh::survey_root);
            reject_extra_args(args)?;
            smoosh::import(&checkout, &output, &smoosh::survey_root())
        }
        Some(command) if command == OsStr::new("verify-smoosh") => {
            let root = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(smoosh::survey_root);
            reject_extra_args(args)?;
            smoosh::verify(&root)
        }
        Some(command) if command == OsStr::new("generate-smoosh-manifest") => {
            let root = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(smoosh::survey_root);
            reject_extra_args(args)?;
            smoosh::generate_manifest(&root)
        }
        Some(command) if command == OsStr::new("run-smoosh") => {
            if smoosh_runner::command(args, smoosh::survey_root())? {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }
        _ => Err(usage().into()),
    }
}

fn usage() -> &'static str {
    "usage: nsh-survey import-oils OILS_CHECKOUT [OUTPUT]\n       nsh-survey verify-oils [ROOT]\n       nsh-survey generate-oils-manifests [ROOT]\n       nsh-survey run-oils [OPTIONS] [ROOT]\n       nsh-survey gate-bash --shell PATH [ROOT]\n\
       nsh-survey build-bash-reference SOURCES OUTPUT\n       nsh-survey calibrate-bash-reference --shell PATH --sources SOURCES [ROOT]\n       nsh-survey verify-bash-reference [--shell PATH] [--sources SOURCES] [ROOT]\n       nsh-survey import-smoosh SMOOSH_CHECKOUT [OUTPUT]\n       nsh-survey verify-smoosh [ROOT]\n       nsh-survey generate-smoosh-manifest [ROOT]\n       nsh-survey run-smoosh [OPTIONS] [ROOT]"
}

fn required_path(value: Option<std::ffi::OsString>, name: &str) -> Result<PathBuf> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {name}; {}", usage()).into())
}

fn reject_extra_args(mut args: env::ArgsOs) -> Result<()> {
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument {:?}; {}", extra, usage()).into());
    }
    Ok(())
}

fn survey_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/surveys/oils")
}

fn read_lock(root: &Path) -> Result<SourceLock> {
    let path = root.join("SOURCE.toml");
    let text = fs::read_to_string(&path)?;
    let lock: SourceLock = toml::from_str(&text)?;
    if lock.schema != 1 {
        return Err(format!("{} has unsupported schema {}", path.display(), lock.schema).into());
    }
    if lock.repository.is_empty()
        || lock.commit.len() != 40
        || lock.tree.len() != 40
        || lock.license != "Apache-2.0"
        || lock.spec_format.is_empty()
    {
        return Err(format!("{} contains an invalid source identity", path.display()).into());
    }
    Ok(lock)
}

fn import_oils(checkout: &Path, output: &Path, metadata_root: &Path) -> Result<()> {
    let lock = read_lock(metadata_root)?;
    verify_checkout(checkout, &lock)?;

    let parent = output
        .parent()
        .ok_or_else(|| format!("output {} has no parent", output.display()))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".oils-import-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let result = (|| {
        generate_import(checkout, metadata_root, &staging, &lock)?;
        verify_import(&staging, &lock)?;
        install_import(&staging, output)
    })();

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    result?;
    println!(
        "imported {} active OSH files and {} cases from {}",
        lock.observed.active_osh_files, lock.observed.active_osh_cases, lock.commit
    );
    Ok(())
}

fn verify_checkout(checkout: &Path, lock: &SourceLock) -> Result<()> {
    let commit = git_value(checkout, &["rev-parse", "HEAD"])?;
    let tree = git_value(checkout, &["rev-parse", "HEAD^{tree}"])?;
    if commit != lock.commit {
        return Err(format!(
            "checkout commit {commit} does not match locked commit {}",
            lock.commit
        )
        .into());
    }
    if tree != lock.tree {
        return Err(format!(
            "checkout tree {tree} does not match locked tree {}",
            lock.tree
        )
        .into());
    }
    Ok(())
}

fn git_value(checkout: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed for {}: {}",
            arguments.join(" "),
            checkout.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn generate_import(
    checkout: &Path,
    metadata_root: &Path,
    staging: &Path,
    lock: &SourceLock,
) -> Result<()> {
    let license_source = checkout.join(&lock.license_path);
    let license_hash = sha256_file(&license_source)?;
    if license_hash != lock.license_sha256 {
        return Err(format!(
            "license hash {license_hash} does not match {}",
            lock.license_sha256
        )
        .into());
    }
    copy_file(&license_source, &staging.join("LICENSE.txt"))?;

    let (all_files, all_cases, active) = discover_specs(checkout)?;
    let active_cases: usize = active.iter().map(|spec| spec.cases).sum();
    let observed = &lock.observed;
    if (all_files, all_cases, active.len(), active_cases)
        != (
            observed.all_files,
            observed.all_cases,
            observed.active_osh_files,
            observed.active_osh_cases,
        )
    {
        return Err(format!(
            "source counts changed: all={all_files}/{all_cases}, active={}/{active_cases}; locked all={}/{}, active={}/{}",
            active.len(),
            observed.all_files,
            observed.all_cases,
            observed.active_osh_files,
            observed.active_osh_cases
        )
        .into());
    }

    for spec in active {
        copy_file(&spec.path, &staging.join("spec").join(spec.file_name))?;
    }
    for fixture in read_fixture_paths(metadata_root)? {
        copy_file(&checkout.join(&fixture), &staging.join(&fixture))?;
    }
    write_hash_inventory(staging)?;

    // Keep a standalone import independently verifiable. These two files are
    // inputs to generation rather than generated payload, so the inventory
    // intentionally covers LICENSE.txt and spec/ only.
    for name in ["SOURCE.toml", "FIXTURES.txt"] {
        copy_file(&metadata_root.join(name), &staging.join(name))?;
    }
    if lock.manifests.is_some() {
        write_oils_manifest(staging, lock)?;
    }
    Ok(())
}

#[derive(Debug)]
struct SpecFile {
    path: PathBuf,
    file_name: String,
    cases: usize,
}

fn discover_specs(checkout: &Path) -> Result<(usize, usize, Vec<SpecFile>)> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(checkout.join("spec"))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.ends_with(".test.sh"))
        {
            paths.push(path);
        }
    }
    paths.sort();

    let mut all_cases = 0;
    let mut active = Vec::new();
    for path in &paths {
        let bytes = fs::read(path)?;
        let cases = count_cases(&bytes);
        all_cases += cases;
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("non-UTF-8 spec filename: {}", path.display()))?;
        if suite_for(file_name, &bytes) == "osh" {
            active.push(SpecFile {
                path: path.clone(),
                file_name: file_name.to_owned(),
                cases,
            });
        }
    }
    Ok((paths.len(), all_cases, active))
}

fn suite_for<'a>(file_name: &'a str, bytes: &'a [u8]) -> &'a str {
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.starts_with(b"#### ") {
            break;
        }
        if let Some(value) = line.strip_prefix(b"## suite:") {
            return std::str::from_utf8(value).unwrap_or("").trim();
        }
    }
    let name = file_name.strip_suffix(".test.sh").unwrap_or(file_name);
    if name.starts_with("ysh-") || name.starts_with("hay") || name.starts_with("tea-") {
        "ysh"
    } else {
        "osh"
    }
}

fn count_cases(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| line.starts_with(b"#### "))
        .count()
}

fn read_fixture_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let text = fs::read_to_string(root.join("FIXTURES.txt"))?;
    let mut fixtures = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = PathBuf::from(line);
        if path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !line.starts_with("spec/")
        {
            return Err(format!("invalid fixture path on line {}: {line}", index + 1).into());
        }
        fixtures.push(path);
    }
    fixtures.sort();
    fixtures.dedup();
    Ok(fixtures)
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_file() {
        return Err(format!("required source file is missing: {}", source.display()).into());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

fn write_hash_inventory(root: &Path) -> Result<()> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.retain(|path| path != Path::new("FILES.sha256"));
    files.sort();
    let mut inventory = String::new();
    for relative in files {
        let hash = sha256_file(&root.join(&relative))?;
        inventory.push_str(&hash);
        inventory.push_str("  ");
        inventory.push_str(&relative.to_string_lossy().replace('\\', "/"));
        inventory.push('\n');
    }
    fs::write(root.join("FILES.sha256"), inventory)?;
    Ok(())
}

fn collect_files(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(root, &path, output)?;
        } else if kind.is_file() {
            output.push(path.strip_prefix(root)?.to_owned());
        } else {
            return Err(
                format!("import source contains non-file entry: {}", path.display()).into(),
            );
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn install_import(staging: &Path, output: &Path) -> Result<()> {
    if output.exists() {
        fs::create_dir_all(output)?;
        for name in [
            "spec",
            "LICENSE.txt",
            "FILES.sha256",
            "SOURCE.toml",
            "FIXTURES.txt",
            "MANIFEST.toml",
        ] {
            let old = output.join(name);
            let new = staging.join(name);
            if !new.exists() {
                if old.is_file() && name == "MANIFEST.toml" {
                    fs::remove_file(old)?;
                }
                continue;
            }
            if old.is_dir() {
                fs::remove_dir_all(&old)?;
            } else if old.exists() {
                fs::remove_file(&old)?;
            }
            fs::rename(new, old)?;
        }
    } else {
        fs::rename(staging, output)?;
    }
    Ok(())
}

fn verify_oils(root: &Path) -> Result<()> {
    let lock = read_lock(root)?;
    verify_import(root, &lock)?;
    if lock.manifests.is_some() {
        verify_oils_manifest(root, &lock)?;
    }
    println!(
        "verified {} imported Oils files and {} active OSH cases at {}",
        inventory(root)?.len(),
        lock.observed.active_osh_cases,
        lock.commit
    );
    Ok(())
}

fn generate_oils_manifests(root: &Path) -> Result<()> {
    let lock = read_lock(root)?;
    if lock.manifests.is_none() {
        return Err(format!(
            "{} has no manifest baselines",
            root.join("SOURCE.toml").display()
        )
        .into());
    }
    write_oils_manifest(root, &lock)?;
    verify_oils_manifest(root, &lock)?;
    println!(
        "generated and verified {}",
        root.join("MANIFEST.toml").display()
    );
    Ok(())
}

fn verify_import(root: &Path, lock: &SourceLock) -> Result<()> {
    let expected = inventory(root)?;
    let mut actual = Vec::new();
    if root.join("LICENSE.txt").is_file() {
        actual.push(PathBuf::from("LICENSE.txt"));
    }
    if root.join("spec").is_dir() {
        collect_files(root, &root.join("spec"), &mut actual)?;
    }
    actual.sort();
    let expected_paths: Vec<_> = expected.keys().cloned().collect();
    if actual != expected_paths {
        let actual_set: BTreeSet<_> = actual.iter().collect();
        let expected_set: BTreeSet<_> = expected_paths.iter().collect();
        let missing: Vec<_> = expected_set.difference(&actual_set).collect();
        let extra: Vec<_> = actual_set.difference(&expected_set).collect();
        return Err(
            format!("import inventory mismatch; missing={missing:?}, extra={extra:?}").into(),
        );
    }
    for (path, wanted) in expected {
        let got = sha256_file(&root.join(&path))?;
        if got != wanted {
            return Err(format!("{} hash {got} does not match {wanted}", path.display()).into());
        }
    }
    if sha256_file(&root.join("LICENSE.txt"))? != lock.license_sha256 {
        return Err("imported license does not match SOURCE.toml".into());
    }

    let (_, cases, specs) = discover_specs(root)?;
    if specs.len() != lock.observed.active_osh_files || cases != lock.observed.active_osh_cases {
        return Err(format!(
            "imported corpus has {}/{} files/cases; expected {}/{}",
            specs.len(),
            cases,
            lock.observed.active_osh_files,
            lock.observed.active_osh_cases
        )
        .into());
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct OilsManifest {
    schema: u32,
    source_commit: String,
    source_tree: String,
    spec_format: String,
    groups: Vec<ManifestGroup>,
    specs: Vec<ManifestSpec>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ManifestGroup {
    id: String,
    label: String,
    selector: String,
    files: usize,
    cases: usize,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ManifestSpec {
    path: String,
    cases: usize,
    compare_shells: Vec<String>,
    groups: Vec<String>,
    qualified_assertions: usize,
    sha256: String,
}

fn write_oils_manifest(root: &Path, lock: &SourceLock) -> Result<()> {
    let manifest = build_oils_manifest(root, lock)?;
    let text = toml::to_string_pretty(&manifest)?;
    fs::write(root.join("MANIFEST.toml"), text)?;
    Ok(())
}

fn verify_oils_manifest(root: &Path, lock: &SourceLock) -> Result<()> {
    let path = root.join("MANIFEST.toml");
    let text = fs::read_to_string(&path)?;
    let actual: OilsManifest = toml::from_str(&text)?;
    let expected = build_oils_manifest(root, lock)?;
    if actual != expected {
        return Err(format!("{} does not match the imported corpus", path.display()).into());
    }
    let canonical = toml::to_string_pretty(&expected)?;
    if text != canonical {
        return Err(format!("{} is not in canonical generated form", path.display()).into());
    }
    Ok(())
}

fn build_oils_manifest(root: &Path, lock: &SourceLock) -> Result<OilsManifest> {
    let expected = lock
        .manifests
        .as_ref()
        .ok_or("SOURCE.toml has no manifest baselines")?;
    let payload = inventory(root)?;
    let (_, cases, active) = discover_specs(root)?;
    if active.len() != lock.observed.active_osh_files || cases != lock.observed.active_osh_cases {
        return Err("cannot manifest an incomplete active OSH corpus".into());
    }

    let mut counts = BTreeMap::<String, ExpectedCount>::new();
    let mut specs = Vec::with_capacity(active.len());
    for spec in active {
        let bytes = fs::read(&spec.path)?;
        let compare_shells = metadata_words(&bytes, "compare_shells")?;
        let groups = manifest_groups(&spec.file_name, &compare_shells);
        for group in &groups {
            let count = counts
                .entry(group.clone())
                .or_insert(ExpectedCount { files: 0, cases: 0 });
            count.files += 1;
            count.cases += spec.cases;
        }
        let relative = PathBuf::from("spec").join(&spec.file_name);
        let hash = payload
            .get(&relative)
            .ok_or_else(|| format!("{} is absent from FILES.sha256", relative.display()))?;
        specs.push(ManifestSpec {
            path: relative.to_string_lossy().replace('\\', "/"),
            cases: spec.cases,
            compare_shells,
            groups,
            qualified_assertions: count_qualified_assertions(&bytes),
            sha256: hash.clone(),
        });
    }

    let definitions = [
        (
            "full",
            "Active OSH corpus",
            "suite is active OSH",
            expected.full,
        ),
        (
            "posix-candidate",
            "Dash-selected POSIX candidate survey",
            "compare_shells contains dash; this is not the normative POSIX oracle",
            expected.posix_candidate,
        ),
        (
            "bash-comparison",
            "Bash comparison survey",
            "compare_shells contains bash or a bash-* version token",
            expected.bash_comparison,
        ),
        (
            "bash-extension",
            "Bash extension survey",
            "Bash comparison selection without dash",
            expected.bash_extension,
        ),
        (
            "bash-named-diagnostic",
            "Named *-bash diagnostic slice",
            "file stem ends with -bash",
            expected.bash_named_diagnostic,
        ),
    ];
    let mut groups = Vec::with_capacity(definitions.len());
    for (id, label, selector, baseline) in definitions {
        let observed = counts
            .get(id)
            .copied()
            .unwrap_or(ExpectedCount { files: 0, cases: 0 });
        if observed != baseline {
            return Err(format!(
                "manifest {id} has {}/{} files/cases; locked baseline is {}/{}",
                observed.files, observed.cases, baseline.files, baseline.cases
            )
            .into());
        }
        groups.push(ManifestGroup {
            id: id.to_owned(),
            label: label.to_owned(),
            selector: selector.to_owned(),
            files: observed.files,
            cases: observed.cases,
        });
    }

    Ok(OilsManifest {
        schema: 1,
        source_commit: lock.commit.clone(),
        source_tree: lock.tree.clone(),
        spec_format: lock.spec_format.clone(),
        groups,
        specs,
    })
}

fn metadata_words(bytes: &[u8], name: &str) -> Result<Vec<String>> {
    let text = std::str::from_utf8(bytes)?;
    let prefix = format!("## {name}:");
    let mut value = None;
    for line in text.lines() {
        if line.starts_with("####") {
            break;
        }
        if let Some(rest) = line.strip_prefix(&prefix) {
            if value.replace(rest.trim()).is_some() {
                return Err(format!("duplicate {name} file metadata").into());
            }
        }
    }
    Ok(value
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect())
}

fn manifest_groups(file_name: &str, compare_shells: &[String]) -> Vec<String> {
    let has_dash = compare_shells.iter().any(|shell| shell == "dash");
    let has_bash = compare_shells.iter().any(|shell| shell.starts_with("bash"));
    let bash_named = file_name
        .strip_suffix(".test.sh")
        .is_some_and(|stem| stem.ends_with("-bash"));
    let mut groups = vec!["full".to_owned()];
    if has_dash {
        groups.push("posix-candidate".to_owned());
    }
    if has_bash {
        groups.push("bash-comparison".to_owned());
    }
    if has_bash && !has_dash {
        groups.push("bash-extension".to_owned());
    }
    if bash_named {
        groups.push("bash-named-diagnostic".to_owned());
    }
    groups
}

fn count_qualified_assertions(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"## "))
        .filter(|line| {
            let token = line
                .split(|byte| byte.is_ascii_whitespace())
                .next()
                .unwrap_or_default();
            token == b"OK"
                || token == b"BUG"
                || token == b"N-I"
                || token.strip_prefix(b"OK-").is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit)
                })
                || token.strip_prefix(b"BUG-").is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit)
                })
        })
        .count()
}

fn inventory(root: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let text = fs::read_to_string(root.join("FILES.sha256"))?;
    let mut result = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let (hash, path) = line
            .split_once("  ")
            .ok_or_else(|| format!("invalid FILES.sha256 line {}", index + 1))?;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("invalid SHA-256 on line {}", index + 1).into());
        }
        let path = PathBuf::from(path);
        if path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(format!("invalid inventory path on line {}", index + 1).into());
        }
        if result.insert(path, hash.to_owned()).is_some() {
            return Err(format!("duplicate inventory path on line {}", index + 1).into());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_metadata_wins_over_filename_default() {
        assert_eq!(
            suite_for("plain.test.sh", b"## suite: disabled\n#### x\n"),
            "disabled"
        );
        assert_eq!(suite_for("ysh-demo.test.sh", b"#### x\n"), "ysh");
        assert_eq!(suite_for("plain.test.sh", b"#### x\n"), "osh");
    }

    #[test]
    fn cases_are_heading_lines_only() {
        assert_eq!(
            count_cases(b"#### one\necho '#### not a heading'\n#### two\n"),
            2
        );
    }

    #[test]
    fn sha256_matches_known_vector() {
        let path = env::temp_dir().join(format!("nsh-survey-sha-{}", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        let result = sha256_file(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(
            result,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn manifest_groups_split_dash_and_bash() {
        let both = vec!["dash".to_owned(), "bash-4.4".to_owned()];
        assert_eq!(
            manifest_groups("plain.test.sh", &both),
            ["full", "posix-candidate", "bash-comparison"]
        );

        let bash = vec!["bash".to_owned()];
        assert_eq!(
            manifest_groups("builtin-bash.test.sh", &bash),
            [
                "full",
                "bash-comparison",
                "bash-extension",
                "bash-named-diagnostic",
            ]
        );
    }

    #[test]
    fn qualified_assertion_records_are_counted_once() {
        let spec = b"#### case\n## BUG bash stdout: bad\n## OK-2 dash STDERR:\ntext\n## END\n";
        assert_eq!(count_qualified_assertions(spec), 2);
    }
}
