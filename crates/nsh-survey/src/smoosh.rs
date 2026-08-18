use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const PAYLOAD_SUFFIXES: &[&str] = &[".test", ".out", ".err", ".ec"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceLock {
    schema: u32,
    repository: String,
    pub(crate) commit: String,
    tree: String,
    license: String,
    license_path: String,
    license_sha256: String,
    suite_path: String,
    observed: Observed,
    manifests: ManifestExpectations,
    pub(crate) timeouts: Timeouts,
    known_hangs: Vec<KnownHang>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observed {
    tests: usize,
    payload_files: usize,
    stdout_oracles: usize,
    stderr_oracles: usize,
    status_oracles: usize,
    orphan_oracles: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestExpectations {
    full: ExpectedCount,
    regular: ExpectedCount,
    known_hang: ExpectedCount,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ExpectedCount {
    tests: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Timeouts {
    pub(crate) default_ms: u64,
    pub(crate) known_hang_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownHang {
    test: String,
    reason: String,
}

#[derive(Debug)]
struct Corpus {
    files: Vec<PathBuf>,
    tests: Vec<String>,
    orphan_oracles: Vec<String>,
    stdout_oracles: usize,
    stderr_oracles: usize,
    status_oracles: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) schema: u32,
    pub(crate) survey: String,
    pub(crate) source_commit: String,
    pub(crate) source_tree: String,
    pub(crate) timeouts: Timeouts,
    pub(crate) groups: Vec<ManifestGroup>,
    pub(crate) tests: Vec<ManifestTest>,
    pub(crate) inactive_payloads: Vec<OracleFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestGroup {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) selector: String,
    pub(crate) tests: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestTest {
    pub(crate) name: String,
    pub(crate) script: OracleFile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdout: Option<OracleFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stderr: Option<OracleFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<OracleFile>,
    pub(crate) expected_status: i32,
    pub(crate) groups: Vec<String>,
    pub(crate) known_hang: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) known_hang_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct OracleFile {
    pub(crate) path: String,
    pub(crate) sha256: String,
}

pub(crate) fn survey_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/surveys/smoosh")
}

pub(crate) fn import(checkout: &Path, output: &Path, metadata_root: &Path) -> Result<()> {
    let lock = read_lock(metadata_root)?;
    verify_checkout(checkout, &lock)?;

    let parent = output
        .parent()
        .ok_or_else(|| format!("output {} has no parent", output.display()))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".smoosh-import-{}", std::process::id()));
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
        "imported {} Smoosh shell tests from {}",
        lock.observed.tests, lock.commit
    );
    Ok(())
}

pub(crate) fn verify(root: &Path) -> Result<()> {
    let lock = read_lock(root)?;
    let _manifest = read_manifest(root, &lock)?;
    println!(
        "verified {} imported Smoosh payload files and {} tests at {}",
        lock.observed.payload_files + 1,
        lock.observed.tests,
        lock.commit
    );
    Ok(())
}

pub(crate) fn generate_manifest(root: &Path) -> Result<()> {
    let lock = read_lock(root)?;
    verify_payload(root, &lock)?;
    write_manifest(root, &lock)?;
    verify_manifest(root, &lock)?;
    println!(
        "generated and verified {}",
        root.join("MANIFEST.toml").display()
    );
    Ok(())
}

pub(crate) fn read_lock(root: &Path) -> Result<SourceLock> {
    let path = root.join("SOURCE.toml");
    let lock: SourceLock = toml::from_str(&fs::read_to_string(&path)?)?;
    if lock.schema != 1
        || lock.repository != "https://github.com/mgree/smoosh.git"
        || lock.commit.len() != 40
        || lock.tree.len() != 40
        || lock.license != "MIT"
        || lock.license_path.is_empty()
        || lock.license_sha256.len() != 64
        || lock.suite_path != "tests/shell"
        || lock.timeouts.default_ms == 0
        || lock.timeouts.known_hang_ms == 0
    {
        return Err(format!("{} contains an invalid source identity", path.display()).into());
    }
    validate_known_hangs(&lock)?;
    Ok(lock)
}

pub(crate) fn read_manifest(root: &Path, lock: &SourceLock) -> Result<Manifest> {
    verify_import(root, lock)?;
    let text = fs::read_to_string(root.join("MANIFEST.toml"))?;
    Ok(toml::from_str(&text)?)
}

fn validate_known_hangs(lock: &SourceLock) -> Result<()> {
    let mut names = BTreeSet::new();
    for entry in &lock.known_hangs {
        if !valid_payload_name(&entry.test, ".test") || entry.reason.trim().is_empty() {
            return Err(format!("invalid known-hang entry for {:?}", entry.test).into());
        }
        if !names.insert(&entry.test) {
            return Err(format!("duplicate known-hang entry for {}", entry.test).into());
        }
    }
    if names.len() != lock.manifests.known_hang.tests {
        return Err(format!(
            "known-hang list has {} tests; lock expects {}",
            names.len(),
            lock.manifests.known_hang.tests
        )
        .into());
    }
    Ok(())
}

fn verify_checkout(checkout: &Path, lock: &SourceLock) -> Result<()> {
    let commit = crate::git_value(checkout, &["rev-parse", "HEAD"])?;
    let tree = crate::git_value(checkout, &["rev-parse", "HEAD^{tree}"])?;
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

fn generate_import(
    checkout: &Path,
    metadata_root: &Path,
    staging: &Path,
    lock: &SourceLock,
) -> Result<()> {
    let license_source = checkout.join(&lock.license_path);
    let license_hash = crate::sha256_file(&license_source)?;
    if license_hash != lock.license_sha256 {
        return Err(format!(
            "license hash {license_hash} does not match {}",
            lock.license_sha256
        )
        .into());
    }
    crate::copy_file(&license_source, &staging.join("LICENSE.txt"))?;

    let source = checkout.join(&lock.suite_path);
    let corpus = discover_corpus(&source)?;
    verify_counts(&corpus, lock, "source")?;
    for relative in corpus.files {
        crate::copy_file(
            &source.join(&relative),
            &staging.join("shell").join(relative),
        )?;
    }
    crate::write_hash_inventory(staging)?;
    for name in ["SOURCE.toml", "README.md"] {
        crate::copy_file(&metadata_root.join(name), &staging.join(name))?;
    }
    write_manifest(staging, lock)?;
    Ok(())
}

fn discover_corpus(directory: &Path) -> Result<Corpus> {
    let mut files = Vec::new();
    let mut tests = BTreeSet::new();
    let mut oracles = BTreeMap::<String, BTreeSet<&'static str>>::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if !kind.is_file() {
            return Err(format!(
                "Smoosh shell corpus contains non-file entry: {}",
                entry.path().display()
            )
            .into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Smoosh shell corpus contains a non-UTF-8 file name")?;
        let suffix = PAYLOAD_SUFFIXES
            .iter()
            .copied()
            .find(|suffix| name.ends_with(suffix))
            .ok_or_else(|| format!("unsupported Smoosh shell payload {name:?}"))?;
        if !valid_payload_name(&name, suffix) {
            return Err(format!("invalid Smoosh shell payload name {name:?}").into());
        }
        let base = name
            .strip_suffix(suffix)
            .expect("selected suffix must strip")
            .to_owned();
        if suffix == ".test" {
            tests.insert(name.clone());
        } else if !oracles.entry(base).or_default().insert(suffix) {
            return Err(format!("duplicate oracle payload {name:?}").into());
        }
        files.push(PathBuf::from(name));
    }
    files.sort();
    let test_bases: BTreeSet<_> = tests
        .iter()
        .map(|name| name.strip_suffix(".test").expect("test suffix"))
        .collect();
    let orphan_oracles = oracles
        .iter()
        .filter(|(base, _)| !test_bases.contains(base.as_str()))
        .flat_map(|(base, suffixes)| suffixes.iter().map(move |suffix| format!("{base}{suffix}")))
        .collect();
    Ok(Corpus {
        files,
        tests: tests.into_iter().collect(),
        orphan_oracles,
        stdout_oracles: oracles.values().filter(|set| set.contains(".out")).count(),
        stderr_oracles: oracles.values().filter(|set| set.contains(".err")).count(),
        status_oracles: oracles.values().filter(|set| set.contains(".ec")).count(),
    })
}

fn valid_payload_name(name: &str, suffix: &str) -> bool {
    name.ends_with(suffix)
        && name.len() > suffix.len()
        && !name.starts_with('.')
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
        })
}

fn verify_counts(corpus: &Corpus, lock: &SourceLock, description: &str) -> Result<()> {
    let got = (
        corpus.tests.len(),
        corpus.files.len(),
        corpus.stdout_oracles,
        corpus.stderr_oracles,
        corpus.status_oracles,
        corpus.orphan_oracles.len(),
    );
    let wanted = (
        lock.observed.tests,
        lock.observed.payload_files,
        lock.observed.stdout_oracles,
        lock.observed.stderr_oracles,
        lock.observed.status_oracles,
        lock.observed.orphan_oracles,
    );
    if got != wanted {
        return Err(format!(
            "{description} counts tests/files/out/err/ec/orphan={got:?}; locked counts={wanted:?}"
        )
        .into());
    }
    let corpus_tests: BTreeSet<_> = corpus.tests.iter().map(String::as_str).collect();
    if let Some(missing) = lock
        .known_hangs
        .iter()
        .find(|entry| !corpus_tests.contains(entry.test.as_str()))
    {
        return Err(format!("known-hang test {} is absent", missing.test).into());
    }
    Ok(())
}

fn install_import(staging: &Path, output: &Path) -> Result<()> {
    if !output.exists() {
        fs::rename(staging, output)?;
        return Ok(());
    }
    fs::create_dir_all(output)?;
    for name in [
        "shell",
        "LICENSE.txt",
        "FILES.sha256",
        "SOURCE.toml",
        "README.md",
        "MANIFEST.toml",
    ] {
        let old = output.join(name);
        let new = staging.join(name);
        if old.is_dir() {
            fs::remove_dir_all(&old)?;
        } else if old.exists() {
            fs::remove_file(&old)?;
        }
        fs::rename(new, old)?;
    }
    Ok(())
}

fn verify_import(root: &Path, lock: &SourceLock) -> Result<()> {
    verify_payload(root, lock)?;
    verify_manifest(root, lock)
}

fn verify_payload(root: &Path, lock: &SourceLock) -> Result<()> {
    let expected = crate::inventory(root)?;
    let mut actual = vec![PathBuf::from("LICENSE.txt")];
    crate::collect_files(root, &root.join("shell"), &mut actual)?;
    actual.sort();
    let expected_paths: Vec<_> = expected.keys().cloned().collect();
    if actual != expected_paths {
        let actual_set: BTreeSet<_> = actual.iter().collect();
        let expected_set: BTreeSet<_> = expected_paths.iter().collect();
        let missing: Vec<_> = expected_set.difference(&actual_set).collect();
        let extra: Vec<_> = actual_set.difference(&expected_set).collect();
        return Err(format!(
            "Smoosh import inventory mismatch; missing={missing:?}, extra={extra:?}"
        )
        .into());
    }
    for (path, wanted) in expected {
        let got = crate::sha256_file(&root.join(&path))?;
        if got != wanted {
            return Err(format!("{} hash {got} does not match {wanted}", path.display()).into());
        }
    }
    if crate::sha256_file(&root.join("LICENSE.txt"))? != lock.license_sha256 {
        return Err("imported Smoosh license does not match SOURCE.toml".into());
    }
    let corpus = discover_corpus(&root.join("shell"))?;
    verify_counts(&corpus, lock, "imported corpus")
}

fn write_manifest(root: &Path, lock: &SourceLock) -> Result<()> {
    let text = toml::to_string_pretty(&build_manifest(root, lock)?)?;
    fs::write(root.join("MANIFEST.toml"), text)?;
    Ok(())
}

fn verify_manifest(root: &Path, lock: &SourceLock) -> Result<()> {
    let path = root.join("MANIFEST.toml");
    let text = fs::read_to_string(&path)?;
    let actual: Manifest = toml::from_str(&text)?;
    let expected = build_manifest(root, lock)?;
    if actual != expected {
        return Err(format!("{} does not match the imported corpus", path.display()).into());
    }
    if text != toml::to_string_pretty(&expected)? {
        return Err(format!("{} is not in canonical generated form", path.display()).into());
    }
    Ok(())
}

fn build_manifest(root: &Path, lock: &SourceLock) -> Result<Manifest> {
    let corpus = discover_corpus(&root.join("shell"))?;
    verify_counts(&corpus, lock, "manifest corpus")?;
    let hangs: BTreeMap<_, _> = lock
        .known_hangs
        .iter()
        .map(|entry| (entry.test.as_str(), entry.reason.as_str()))
        .collect();
    let mut tests = Vec::with_capacity(corpus.tests.len());
    for name in corpus.tests {
        let base = name.strip_suffix(".test").expect("test suffix");
        let known_hang_reason = hangs.get(name.as_str()).map(|reason| (*reason).to_owned());
        let known_hang = known_hang_reason.is_some();
        let status = oracle_if_present(root, base, ".ec")?;
        let expected_status = match &status {
            Some(oracle) => parse_status(&root.join(&oracle.path))?,
            None => 0,
        };
        tests.push(ManifestTest {
            name: name.clone(),
            script: oracle(root, &format!("shell/{name}"))?,
            stdout: oracle_if_present(root, base, ".out")?,
            stderr: oracle_if_present(root, base, ".err")?,
            status,
            expected_status,
            groups: vec![
                "full".to_owned(),
                if known_hang { "known-hang" } else { "regular" }.to_owned(),
            ],
            known_hang,
            known_hang_reason,
        });
    }
    let counts = |group: &str| {
        tests
            .iter()
            .filter(|test| test.groups.iter().any(|candidate| candidate == group))
            .count()
    };
    let full = counts("full");
    let regular = counts("regular");
    let known_hang = counts("known-hang");
    if full != lock.manifests.full.tests
        || regular != lock.manifests.regular.tests
        || known_hang != lock.manifests.known_hang.tests
    {
        return Err(format!(
            "manifest counts full/regular/known-hang={full}/{regular}/{known_hang}; locked counts={}/{}/{}",
            lock.manifests.full.tests,
            lock.manifests.regular.tests,
            lock.manifests.known_hang.tests
        )
        .into());
    }
    Ok(Manifest {
        schema: 1,
        survey: "smoosh-shell-posix".to_owned(),
        source_commit: lock.commit.clone(),
        source_tree: lock.tree.clone(),
        timeouts: lock.timeouts,
        groups: vec![
            ManifestGroup {
                id: "full".to_owned(),
                label: "All pinned Smoosh shell tests".to_owned(),
                selector: "every imported .test script".to_owned(),
                tests: full,
            },
            ManifestGroup {
                id: "regular".to_owned(),
                label: "Bounded ordinary Smoosh tests".to_owned(),
                selector: "full minus the reviewed known-hang set".to_owned(),
                tests: regular,
            },
            ManifestGroup {
                id: "known-hang".to_owned(),
                label: "Legacy known-hang and translation-hazard tests".to_owned(),
                selector: "the reviewed known_hangs entries in SOURCE.toml".to_owned(),
                tests: known_hang,
            },
        ],
        tests,
        inactive_payloads: corpus
            .orphan_oracles
            .iter()
            .map(|name| oracle(root, &format!("shell/{name}")))
            .collect::<Result<_>>()?,
    })
}

fn oracle_if_present(root: &Path, base: &str, suffix: &str) -> Result<Option<OracleFile>> {
    let path = format!("shell/{base}{suffix}");
    if root.join(&path).is_file() {
        Ok(Some(oracle(root, &path)?))
    } else {
        Ok(None)
    }
}

fn oracle(root: &Path, path: &str) -> Result<OracleFile> {
    Ok(OracleFile {
        path: path.to_owned(),
        sha256: crate::sha256_file(&root.join(path))?,
    })
}

fn parse_status(path: &Path) -> Result<i32> {
    let bytes = fs::read(path)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    let text = std::str::from_utf8(&bytes)?.trim();
    let status: i32 = text
        .parse()
        .map_err(|error| format!("invalid status oracle {}: {error}", path.display()))?;
    if !(0..=255).contains(&status) {
        return Err(format!("status oracle {} is outside 0..=255", path.display()).into());
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_names_are_flat_and_constrained() {
        assert!(valid_payload_name("semantics.tilde.sep.test", ".test"));
        assert!(valid_payload_name("builtin.kill0_+5.test", ".test"));
        assert!(!valid_payload_name("../escape.test", ".test"));
        assert!(!valid_payload_name(".hidden.test", ".test"));
    }

    #[test]
    fn checked_in_manifest_verifies() {
        let root = survey_root();
        if root.join("MANIFEST.toml").is_file() {
            verify(&root).unwrap();
        }
    }

    #[test]
    fn status_oracle_is_trimmed_and_bounded() {
        let root = std::env::temp_dir().join(format!("nsh-smoosh-status-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("case.ec");
        fs::write(&path, b"127\n").unwrap();
        assert_eq!(parse_status(&path).unwrap(), 127);
        fs::write(&path, b"256\n").unwrap();
        assert!(parse_status(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inventory_paths_cannot_escape() {
        let path = PathBuf::from("shell/example.test");
        assert!(
            path.components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        );
        let path = PathBuf::from("shell/../escape.test");
        assert!(
            !path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        );
    }

    #[test]
    fn suffix_matching_prefers_complete_suffixes() {
        assert_eq!(
            PAYLOAD_SUFFIXES
                .iter()
                .find(|suffix| std::ffi::OsStr::new("x.test")
                    .to_string_lossy()
                    .ends_with(**suffix)),
            Some(&".test")
        );
    }
}
