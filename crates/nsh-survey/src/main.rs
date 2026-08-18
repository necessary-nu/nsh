use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

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
}

#[derive(Debug, Deserialize)]
struct Observed {
    all_files: usize,
    all_cases: usize,
    active_osh_files: usize,
    active_osh_cases: usize,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("nsh-survey: {error}");
        std::process::exit(1);
    }
}

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
        _ => Err(usage().into()),
    }
}

fn usage() -> &'static str {
    "usage: nsh-survey import-oils OILS_CHECKOUT [OUTPUT]\n       nsh-survey verify-oils [ROOT]"
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
        ] {
            let old = output.join(name);
            if old.is_dir() {
                fs::remove_dir_all(&old)?;
            } else if old.exists() {
                fs::remove_file(&old)?;
            }
            let new = staging.join(name);
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
    println!(
        "verified {} imported Oils files and {} active OSH cases at {}",
        inventory(root)?.len(),
        lock.observed.active_osh_cases,
        lock.commit
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
}
