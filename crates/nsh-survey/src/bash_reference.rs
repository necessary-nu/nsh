use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::oils_runner::{
    CatalogCase, ReferenceCase, ReferenceOutcome, ReferenceReport, ReferenceTotals,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const PROFILE_FILE: &str = "BASH_REFERENCE.toml";
const CASES_FILE: &str = "BASH_REFERENCE_CASES.json";
const BUILD_RECEIPT_FILE: &str = "BUILD.toml";
const ORACLE_VERSION: &str = "5.3.15(1)-release";
const RELEASE: &str = "5.3";
const PATCH_LEVEL: u32 = 15;
const SOURCE_DATE_EPOCH: u64 = 1_753_833_600;
const SOURCE_FILE: &str = "bash-5.3.tar.gz";
const SOURCE_URL: &str = "https://ftp.gnu.org/gnu/bash/bash-5.3.tar.gz";
const SOURCE_SIGNATURE_URL: &str = "https://ftp.gnu.org/gnu/bash/bash-5.3.tar.gz.sig";
const SOURCE_SHA256: &str = "0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba";
const BUILD_SYSTEM: &str = "x86_64-pc-linux-gnu";
const BUILD_PREFIX: &str = "/opt/nsh-bash-5.3.15";
const CFLAGS_TEMPLATE: &str = "-O2 -g0 -ffile-prefix-map={output}=/usr/src/bash-5.3.15";
const CC_VERSION: &str = "cc (Debian 14.2.0-19) 14.2.0";
const MAKE_VERSION: &str = "GNU Make 4.4.1";
const TAR_VERSION: &str = "tar (GNU tar) 1.35";
const PATCH_VERSION: &str = "GNU patch 2.8";
const BISON_VERSION: &str = "bison (GNU Bison) 3.8.2";
const LIBC_VERSION: &str = "ldd (Debian GLIBC 2.41-12+deb13u3) 2.41";
const KERNEL: &str = "Linux 6.12.100+deb13-amd64 x86_64";
const FIXED_PATH: &str = "/usr/bin:/bin";
const GROUPS: [&str; 3] = ["bash-comparison", "bash-extension", "bash-named-diagnostic"];

const PATCHES: [(&str, &str); 15] = [
    (
        "bash53-001",
        "1f608434364af86b9b45c8b0ea3fb3b165fb830d27697e6cdfc7ac17dee3287f",
    ),
    (
        "bash53-002",
        "e385548a00130765ec7938a56fbdca52447ab41fabc95a25f19ade527e282001",
    ),
    (
        "bash53-003",
        "f245d9c7dc3f5a20d84b53d249334747940936f09dc97e1dcb89fc3ab37d60ed",
    ),
    (
        "bash53-004",
        "9591d245045529f32f0812f94180b9d9ce9023f5a765c039b852e5dfc99747d0",
    ),
    (
        "bash53-005",
        "cca1ef52dbbf433bc98e33269b64b2c814028efe2538be1e2c9a377da90bc99d",
    ),
    (
        "bash53-006",
        "29119addefed8eff91ae37fd51822c31780ee30d4a28376e96002706c995ff10",
    ),
    (
        "bash53-007",
        "c0976bbfffa1453c7cfdd62058f206a318568ff2d690f5d4fa048793fa3eb299",
    ),
    (
        "bash53-008",
        "097cd723cbfb8907674ac32214063a3fd85282657ec5b4e544d2c0f719653fb4",
    ),
    (
        "bash53-009",
        "eee30fe78a4b0cb2fe20e010e00308899cfc613e0774ebb3c8557a1552f24f8c",
    ),
    (
        "bash53-010",
        "cf76f1cce2ea300c18bff9f002d21f280cc931acd17c28518110b93fe6e72569",
    ),
    (
        "bash53-011",
        "0298df8f5ea2a31d3be43ed7d269c5b3c7c342dd5b570bea7f64d66dcbbe7531",
    ),
    (
        "bash53-012",
        "d71379b39bebaedaf123414414e77fb458a0a43b9ad3116594c6df7ca6754573",
    ),
    (
        "bash53-013",
        "042f9cda967e24bf4211944697441e93d06ff42b4b998629a98a1b249279f200",
    ),
    (
        "bash53-014",
        "bd4360b401d38507e358783dcad8536a99c6789f0d3a5bd0cfb8c4a34144696c",
    ),
    (
        "bash53-015",
        "55b79ceee2fc27f6767eed697e939a7eb2fe2a28c01556bd75f18d581014f46e",
    ),
];

fn configure_args() -> Vec<String> {
    vec![
        format!("--prefix={BUILD_PREFIX}"),
        "--without-bash-malloc".to_owned(),
        "--disable-nls".to_owned(),
    ]
}

pub(crate) fn build_command(mut args: env::ArgsOs) -> Result<()> {
    let sources = required_path(args.next(), "SOURCES")?;
    let output = required_path(args.next(), "OUTPUT")?;
    reject_extra_args(args)?;
    build(&sources, &output)
}

pub(crate) fn calibrate_command(mut args: env::ArgsOs, default_root: PathBuf) -> Result<()> {
    let mut shell = None;
    let mut sources = None;
    let mut root = None;
    let mut overwrite = false;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--shell") => shell = Some(required_path(args.next(), "--shell PATH")?),
            Some("--sources") => sources = Some(required_path(args.next(), "--sources SOURCES")?),
            Some("--overwrite-a-changed-file") => overwrite = true,
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown calibrate option {value:?}").into());
            }
            _ if root.is_none() => root = Some(PathBuf::from(argument)),
            _ => return Err("calibrate-bash-reference accepts only one ROOT".into()),
        }
    }
    let shell = shell.ok_or("calibrate-bash-reference requires --shell PATH")?;
    let sources = sources.ok_or("calibrate-bash-reference requires --sources SOURCES")?;
    calibrate(&root.unwrap_or(default_root), &sources, &shell, overwrite)
}

pub(crate) fn verify_command(mut args: env::ArgsOs, default_root: PathBuf) -> Result<()> {
    let mut shell = None;
    let mut sources = None;
    let mut root = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--shell") => shell = Some(required_path(args.next(), "--shell PATH")?),
            Some("--sources") => sources = Some(required_path(args.next(), "--sources SOURCES")?),
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown verify option {value:?}").into());
            }
            _ if root.is_none() => root = Some(PathBuf::from(argument)),
            _ => return Err("verify-bash-reference accepts only one ROOT".into()),
        }
    }
    let root = root.unwrap_or(default_root);
    let verified = verify(&root, sources.as_deref(), shell.as_deref())?;
    println!(
        "verified Bash {} reference: {} eligible, {} explicitly excluded",
        verified.version, verified.eligible, verified.excluded
    );
    Ok(())
}

/// The pinned eligibility calibration, as the closure gate needs it: the
/// eligible case ids, and the disposition recorded for every case the
/// reference build does not itself pass.
// [spec:nsh:req:compat.bash.reference-profile]
pub(crate) fn calibration(
    root: &Path,
) -> Result<(BTreeSet<String>, std::collections::BTreeMap<String, String>)> {
    let cases: CaseManifest = serde_json::from_str(&fs::read_to_string(root.join(CASES_FILE))?)?;
    if cases.schema != 1 {
        return Err(format!("{CASES_FILE} has unsupported schema").into());
    }
    let eligible = cases.eligible.iter().cloned().collect::<BTreeSet<_>>();
    let mut excluded = std::collections::BTreeMap::new();
    for case in &cases.excluded {
        excluded.insert(
            case.id.clone(),
            disposition_label(case.disposition).to_owned(),
        );
    }
    Ok((eligible, excluded))
}

const fn disposition_label(disposition: Disposition) -> &'static str {
    match disposition {
        Disposition::ReferenceFailure => "reference-failure",
        Disposition::Unsupported => "unsupported",
        Disposition::KnownUpstreamBug => "known-upstream-bug",
        Disposition::Timeout => "timeout",
        Disposition::HarnessError => "harness-error",
        Disposition::VersionInapplicable => "version-inapplicable",
    }
}

fn required_path(value: Option<OsString>, name: &str) -> Result<PathBuf> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {name}").into())
}

fn reject_extra_args(mut args: env::ArgsOs) -> Result<()> {
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument {extra:?}").into());
    }
    Ok(())
}

fn build(sources: &Path, output: &Path) -> Result<()> {
    verify_source_files(sources)?;
    verify_toolchain()?;
    if output.exists() {
        if !output.is_dir() || fs::read_dir(output)?.next().is_some() {
            return Err(format!(
                "build output {} must be an empty directory",
                output.display()
            )
            .into());
        }
    } else {
        fs::create_dir_all(output)?;
    }
    let sources = fs::canonicalize(sources)?;
    let output = fs::canonicalize(output)?;
    let output_text = output.to_str().ok_or("build output path must be UTF-8")?;
    if output_text
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte == b':')
    {
        return Err("build output path must not contain whitespace or ':'".into());
    }

    let mut tar = Command::new("/usr/bin/tar");
    tar.args([OsStr::new("-xzf"), sources.join(SOURCE_FILE).as_os_str()])
        .arg("-C")
        .arg(&output);
    run_checked(&mut tar, "extract Bash source")?;

    let source = output.join("bash-5.3");
    for (name, _) in PATCHES {
        let patch_input = fs::File::open(sources.join(name))?;
        let mut patch = Command::new("/usr/bin/patch");
        patch
            .args(["-s", "-p0"])
            .current_dir(&source)
            .stdin(Stdio::from(patch_input));
        run_checked(&mut patch, &format!("apply {name}"))?;
    }
    let patchlevel = fs::read_to_string(source.join("patchlevel.h"))?;
    if !patchlevel
        .lines()
        .any(|line| line.trim() == "#define PATCHLEVEL 15")
    {
        return Err("patched source does not declare PATCHLEVEL 15".into());
    }

    let home = output.join("home");
    fs::create_dir(&home)?;
    let cflags = CFLAGS_TEMPLATE.replace("{output}", output_text);
    let mut configure = Command::new(source.join("configure"));
    configure.current_dir(&source).args(configure_args());
    fixed_build_environment(&mut configure, &home, &cflags);
    run_checked(&mut configure, "configure Bash")?;

    let mut make = Command::new("/usr/bin/make");
    make.current_dir(&source).args(["-s", "-j2"]);
    fixed_build_environment(&mut make, &home, &cflags);
    run_checked(&mut make, "build Bash")?;

    let binary = source.join("bash");
    let version = bash_version(&binary)?;
    if version != ORACLE_VERSION {
        return Err(format!("built Bash reports {version:?}, expected {ORACLE_VERSION:?}").into());
    }
    let binary_sha256 = crate::sha256_file(&binary)?;
    let receipt = BuildReceipt {
        schema: 1,
        release: RELEASE.to_owned(),
        patch_level: PATCH_LEVEL,
        version,
        source_sha256: SOURCE_SHA256.to_owned(),
        binary: "bash-5.3/bash".to_owned(),
        binary_sha256: binary_sha256.clone(),
        build_system: BUILD_SYSTEM.to_owned(),
        configure_args: configure_args(),
        cflags,
        make_args: vec!["-s".to_owned(), "-j2".to_owned()],
        source_date_epoch: SOURCE_DATE_EPOCH,
        tools: pinned_tools(),
    };
    fs::write(
        output.join(BUILD_RECEIPT_FILE),
        toml::to_string_pretty(&receipt)?,
    )?;
    println!("built {} ({binary_sha256})", binary.display());
    Ok(())
}

fn fixed_build_environment(command: &mut Command, home: &Path, cflags: &str) {
    command
        .env_clear()
        .env("PATH", FIXED_PATH)
        .env("HOME", home)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env("SOURCE_DATE_EPOCH", SOURCE_DATE_EPOCH.to_string())
        .env("CC", "/usr/bin/cc")
        .env("CFLAGS", cflags);
}

fn run_checked(command: &mut Command, description: &str) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("{description} failed with {status}").into());
    }
    Ok(())
}

fn bash_version(shell: &Path) -> Result<String> {
    let output = Command::new(shell)
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "printf '%s\\n' \"$BASH_VERSION\"",
        ])
        .env_clear()
        .env("PATH", FIXED_PATH)
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .output()?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(format!(
            "cannot identify Bash reference: status={}, stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn verify_source_files(sources: &Path) -> Result<()> {
    verify_digest(&sources.join(SOURCE_FILE), SOURCE_SHA256)?;
    for (name, digest) in PATCHES {
        verify_digest(&sources.join(name), digest)?;
    }
    Ok(())
}

fn verify_digest(path: &Path, expected: &str) -> Result<()> {
    let actual = crate::sha256_file(path)
        .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    if actual != expected {
        return Err(format!(
            "{} has SHA-256 {actual}, expected {expected}",
            path.display()
        )
        .into());
    }
    Ok(())
}

fn verify_toolchain() -> Result<()> {
    let expected = [
        ("/usr/bin/cc", &["--version"][..], CC_VERSION),
        ("/usr/bin/make", &["--version"][..], MAKE_VERSION),
        ("/usr/bin/tar", &["--version"][..], TAR_VERSION),
        ("/usr/bin/patch", &["--version"][..], PATCH_VERSION),
        ("/usr/bin/bison", &["--version"][..], BISON_VERSION),
        ("/usr/bin/ldd", &["--version"][..], LIBC_VERSION),
    ];
    for (program, arguments, wanted) in expected {
        let actual = command_first_line(program, arguments)?;
        if actual != wanted {
            return Err(format!(
                "{program} reports {actual:?}, pinned environment requires {wanted:?}"
            )
            .into());
        }
    }
    let kernel = command_line("/usr/bin/uname", &["-srm"])?;
    if kernel != KERNEL {
        return Err(format!("kernel is {kernel:?}, pinned environment requires {KERNEL:?}").into());
    }
    Ok(())
}

fn command_first_line(program: &str, arguments: &[&str]) -> Result<String> {
    Ok(command_line(program, arguments)?
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned())
}

fn command_line(program: &str, arguments: &[&str]) -> Result<String> {
    let output = Command::new(program).args(arguments).output()?;
    if !output.status.success() {
        return Err(format!("{program} failed with {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn pinned_tools() -> ToolProfile {
    ToolProfile {
        cc: CC_VERSION.to_owned(),
        make: MAKE_VERSION.to_owned(),
        tar: TAR_VERSION.to_owned(),
        patch: PATCH_VERSION.to_owned(),
        bison: BISON_VERSION.to_owned(),
        libc: LIBC_VERSION.to_owned(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BuildReceipt {
    schema: u32,
    release: String,
    patch_level: u32,
    version: String,
    source_sha256: String,
    binary: String,
    binary_sha256: String,
    build_system: String,
    configure_args: Vec<String>,
    cflags: String,
    make_args: Vec<String>,
    source_date_epoch: u64,
    tools: ToolProfile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolProfile {
    cc: String,
    make: String,
    tar: String,
    patch: String,
    bison: String,
    libc: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReferenceProfile {
    schema: u32,
    name: String,
    release: String,
    patch_level: u32,
    version: String,
    source: SourceProfile,
    patches: Vec<PatchProfile>,
    build: BuildProfile,
    execution: ExecutionProfile,
    oils: OilsProfile,
    calibration: CalibrationProfile,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceProfile {
    archive: String,
    url: String,
    signature_url: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PatchProfile {
    level: u32,
    file: String,
    url: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BuildProfile {
    system: String,
    prefix: String,
    configure_args: Vec<String>,
    cc: String,
    cflags_template: String,
    make_args: Vec<String>,
    source_date_epoch: u64,
    environment: BTreeMap<String, String>,
    tools: ToolProfile,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionProfile {
    kernel: String,
    containment: String,
    locale: String,
    timezone: String,
    base_path: String,
    environment_is_cleared: bool,
    environment: BTreeMap<String, String>,
    unset: Vec<String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OilsProfile {
    commit: String,
    tree: String,
    spec_format: String,
    manifest: String,
    manifest_sha256: String,
    expectation_shell: String,
    groups: Vec<String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CalibrationProfile {
    date: String,
    binary_sha256: String,
    timeout_ms: u64,
    case_manifest: String,
    case_manifest_sha256: String,
    eligible: usize,
    excluded: usize,
    groups: Vec<GroupProfile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct GroupProfile {
    id: String,
    selected: usize,
    eligible: usize,
    reference_failure: usize,
    unsupported: usize,
    known_upstream_bug: usize,
    timeout: usize,
    harness_error: usize,
    version_inapplicable: usize,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseManifest {
    schema: u32,
    oracle_version: String,
    oracle_binary_sha256: String,
    oils_commit: String,
    oils_tree: String,
    eligible: Vec<String>,
    excluded: Vec<ExcludedCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ExcludedCase {
    id: String,
    spec: String,
    index: usize,
    line: usize,
    description: String,
    groups: Vec<String>,
    disposition: Disposition,
    reference_outcome: RecordedOutcome,
    status: Option<i32>,
    qualifier: Option<String>,
    difference_fields: Vec<String>,
    note: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Disposition {
    ReferenceFailure,
    Unsupported,
    KnownUpstreamBug,
    Timeout,
    HarnessError,
    VersionInapplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum RecordedOutcome {
    Fail,
    Unsupported,
    KnownBug,
    Timeout,
    Error,
}

#[derive(Debug)]
pub(crate) struct VerifiedReference {
    pub(crate) version: String,
    pub(crate) eligible: usize,
    pub(crate) excluded: usize,
}

/// The two tracked files a calibration replaces.
///
/// Both are registers this repository reads as authority --
/// `BASH_REFERENCE_CASES.json` decides which cases the closure gate may
/// judge at all -- so a calibration that quietly wrote over somebody
/// else's re-record would move the gate's own definition of expected.
fn guard_generated_files(root: &Path, overwrite: bool) -> Result<()> {
    for name in [CASES_FILE, PROFILE_FILE] {
        crate::provenance::guard_generated(&root.join(name), overwrite)?;
    }
    Ok(())
}

fn calibrate(root: &Path, sources: &Path, shell: &Path, overwrite: bool) -> Result<()> {
    /* Asked before the calibration rather than after it: the run is two
     * whole survey groups, and a refusal that arrives at the write has
     * already spent all of them. Asked again at each write, because the
     * window between is exactly long enough for another session to
     * re-record one of these. */
    guard_generated_files(root, overwrite)?;
    verify_source_files(sources)?;
    verify_toolchain()?;
    let root = fs::canonicalize(root)?;
    let shell = fs::canonicalize(shell)?;
    let receipt_root = shell
        .parent()
        .and_then(Path::parent)
        .ok_or("Bash binary must be OUTPUT/bash-5.3/bash from build-bash-reference")?;
    let receipt_path = receipt_root.join(BUILD_RECEIPT_FILE);
    let receipt_text = fs::read_to_string(&receipt_path).map_err(|error| {
        format!(
            "cannot read build receipt {}: {error}; use build-bash-reference",
            receipt_path.display()
        )
    })?;
    let receipt: BuildReceipt = toml::from_str(&receipt_text)?;
    verify_build_receipt(&receipt, receipt_root, &shell)?;

    let lock = crate::read_lock(&root)?;
    crate::verify_import(&root, &lock)?;
    crate::verify_oils_manifest(&root, &lock)?;
    let manifest_text = fs::read_to_string(root.join("MANIFEST.toml"))?;
    let manifest: crate::OilsManifest = toml::from_str(&manifest_text)?;
    if manifest.source_commit != lock.commit || manifest.source_tree != lock.tree {
        return Err("Oils source lock and manifest disagree".into());
    }
    let version = bash_version(&shell)?;
    if version != ORACLE_VERSION {
        return Err(
            format!("reference shell reports {version:?}, expected {ORACLE_VERSION:?}").into(),
        );
    }

    let mut reports = Vec::new();
    for group in GROUPS {
        println!("calibrating Oils group {group} against Bash {ORACLE_VERSION}");
        reports.push(crate::oils_runner::run_reference_group(
            &root, &manifest, &shell, group,
        )?);
    }
    let catalog = crate::oils_runner::bash_case_catalog(&root, &manifest)?;
    let (case_manifest, groups, containment, timeout_ms) =
        build_case_manifest(&manifest, &catalog, &reports, &receipt.binary_sha256)?;
    let case_text = format!("{}\n", serde_json::to_string_pretty(&case_manifest)?);
    let case_sha256 = sha256_bytes(case_text.as_bytes());
    guard_generated_files(&root, overwrite)?;
    fs::write(root.join(CASES_FILE), &case_text)?;

    let profile = ReferenceProfile {
        schema: 1,
        name: "GNU Bash 5.3 differential reference".to_owned(),
        release: RELEASE.to_owned(),
        patch_level: PATCH_LEVEL,
        version,
        source: expected_source_profile(),
        patches: expected_patch_profiles(),
        build: expected_build_profile(),
        execution: expected_execution_profile(containment),
        oils: OilsProfile {
            commit: manifest.source_commit,
            tree: manifest.source_tree,
            spec_format: manifest.spec_format,
            manifest: "MANIFEST.toml".to_owned(),
            manifest_sha256: sha256_bytes(manifest_text.as_bytes()),
            expectation_shell: "bash".to_owned(),
            groups: GROUPS.into_iter().map(str::to_owned).collect(),
        },
        calibration: CalibrationProfile {
            date: "2026-08-19".to_owned(),
            binary_sha256: receipt.binary_sha256,
            timeout_ms,
            case_manifest: CASES_FILE.to_owned(),
            case_manifest_sha256: case_sha256,
            eligible: case_manifest.eligible.len(),
            excluded: case_manifest.excluded.len(),
            groups,
        },
    };
    guard_generated_files(&root, overwrite)?;
    fs::write(root.join(PROFILE_FILE), toml::to_string_pretty(&profile)?)?;
    verify(&root, Some(sources), Some(&shell))?;
    println!(
        "calibrated Bash {}: {} eligible, {} explicitly excluded",
        ORACLE_VERSION,
        case_manifest.eligible.len(),
        case_manifest.excluded.len()
    );
    Ok(())
}

fn verify_build_receipt(receipt: &BuildReceipt, root: &Path, shell: &Path) -> Result<()> {
    if receipt.schema != 1
        || receipt.release != RELEASE
        || receipt.patch_level != PATCH_LEVEL
        || receipt.version != ORACLE_VERSION
        || receipt.source_sha256 != SOURCE_SHA256
        || receipt.build_system != BUILD_SYSTEM
        || receipt.configure_args != configure_args()
        || receipt.make_args != ["-s", "-j2"]
        || receipt.source_date_epoch != SOURCE_DATE_EPOCH
        || receipt.tools != pinned_tools()
    {
        return Err("Bash build receipt does not match the pinned build profile".into());
    }
    let output_text = root.to_str().ok_or("Bash build root must be UTF-8")?;
    if receipt.cflags != CFLAGS_TEMPLATE.replace("{output}", output_text) {
        return Err("Bash build receipt has unexpected CFLAGS".into());
    }
    let receipt_binary = fs::canonicalize(root.join(&receipt.binary))?;
    if receipt_binary != shell {
        return Err("Bash build receipt names a different binary".into());
    }
    verify_digest(shell, &receipt.binary_sha256)
}

fn build_case_manifest(
    manifest: &crate::OilsManifest,
    catalog: &[CatalogCase],
    reports: &[ReferenceReport],
    binary_sha256: &str,
) -> Result<(CaseManifest, Vec<GroupProfile>, String, u64)> {
    let catalog_by_id = unique_catalog(catalog)?;
    if reports.len() != GROUPS.len() {
        return Err(format!("expected {} reference reports", GROUPS.len()).into());
    }
    for report in reports {
        validate_reference_report(report, &catalog_by_id, &manifest.source_commit)?;
        if !GROUPS.contains(&report.group.as_str()) {
            return Err(format!("unexpected reference group {}", report.group).into());
        }
        if report.shell_sha256 != binary_sha256 {
            return Err(format!(
                "{} used binary {}, build receipt records {}",
                report.group, report.shell_sha256, binary_sha256
            )
            .into());
        }
    }
    let report_groups = reports
        .iter()
        .map(|report| report.group.as_str())
        .collect::<BTreeSet<_>>();
    if report_groups != GROUPS.into_iter().collect() {
        return Err("reference reports do not cover each pinned group exactly once".into());
    }
    let first = &reports[0];
    if reports.iter().any(|report| {
        report.containment != first.containment || report.timeout_ms != first.timeout_ms
    }) {
        return Err("reference groups used different containment or timeout settings".into());
    }
    let comparison = reports
        .iter()
        .find(|report| report.group == "bash-comparison")
        .ok_or("missing bash-comparison report")?;
    let comparison_by_id = unique_report_cases(comparison)?;
    for report in reports
        .iter()
        .filter(|report| report.group != "bash-comparison")
    {
        for case in &report.cases {
            let baseline = comparison_by_id
                .get(&case.id)
                .ok_or_else(|| format!("{} is absent from bash-comparison", case.id))?;
            if !same_observation(case, baseline) {
                return Err(format!(
                    "{} produced a different observation in {} and bash-comparison",
                    case.id, report.group
                )
                .into());
            }
        }
    }

    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    for (id, catalog_case) in &catalog_by_id {
        let observation = comparison_by_id
            .get(id)
            .ok_or_else(|| format!("reference did not execute {id}"))?;
        if observation.outcome == ReferenceOutcome::Pass {
            eligible.push(id.clone());
        } else {
            let (disposition, reference_outcome) = disposition(observation, catalog_case)?;
            excluded.push(ExcludedCase {
                id: id.clone(),
                spec: catalog_case.spec.clone(),
                index: catalog_case.index,
                line: catalog_case.line,
                description: catalog_case.description.clone(),
                groups: catalog_case.groups.clone(),
                disposition,
                reference_outcome,
                status: observation.status,
                qualifier: observation.qualifier.clone(),
                difference_fields: observation.difference_fields.clone(),
                note: observation.note.clone(),
            });
        }
    }
    let case_manifest = CaseManifest {
        schema: 1,
        oracle_version: ORACLE_VERSION.to_owned(),
        oracle_binary_sha256: binary_sha256.to_owned(),
        oils_commit: manifest.source_commit.clone(),
        oils_tree: manifest.source_tree.clone(),
        eligible,
        excluded,
    };
    let groups = build_group_profiles(catalog, &case_manifest, reports)?;
    Ok((
        case_manifest,
        groups,
        first.containment.clone(),
        first.timeout_ms,
    ))
}

fn unique_catalog(catalog: &[CatalogCase]) -> Result<BTreeMap<String, &CatalogCase>> {
    let mut result = BTreeMap::new();
    for case in catalog {
        if result.insert(case.id.clone(), case).is_some() {
            return Err(format!("duplicate catalog case {}", case.id).into());
        }
    }
    Ok(result)
}

fn unique_report_cases(report: &ReferenceReport) -> Result<BTreeMap<String, &ReferenceCase>> {
    let mut result = BTreeMap::new();
    for case in &report.cases {
        if result.insert(case.id.clone(), case).is_some() {
            return Err(format!("duplicate {} case {}", report.group, case.id).into());
        }
    }
    Ok(result)
}

fn validate_reference_report(
    report: &ReferenceReport,
    catalog: &BTreeMap<String, &CatalogCase>,
    source_commit: &str,
) -> Result<()> {
    if report.source_commit != source_commit {
        return Err(format!("{} used a different Oils commit", report.group).into());
    }
    let expected = catalog
        .values()
        .filter(|case| case.groups.iter().any(|group| group == &report.group))
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    let observed = unique_report_cases(report)?;
    let observed_ids = observed.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if observed_ids != expected {
        return Err(format!("{} did not execute its exact manifest", report.group).into());
    }
    for (id, case) in observed {
        let expected_case = catalog.get(&id).expect("set equality checked above");
        if case.spec != expected_case.spec
            || case.index != expected_case.index
            || case.line != expected_case.line
            || case.description != expected_case.description
        {
            return Err(format!("{id} metadata differs from the pinned corpus").into());
        }
    }
    let totals = observed_totals(&report.cases);
    if report.totals != totals
        || report.totals.selected != expected.len()
        || report.totals.executed != expected.len()
    {
        return Err(format!("{} result totals are inconsistent", report.group).into());
    }
    Ok(())
}

fn observed_totals(cases: &[ReferenceCase]) -> ReferenceTotals {
    let mut totals = ReferenceTotals {
        selected: cases.len(),
        executed: cases.len(),
        ..ReferenceTotals::default()
    };
    for case in cases {
        match case.outcome {
            ReferenceOutcome::Pass => totals.pass += 1,
            ReferenceOutcome::Fail => totals.fail += 1,
            ReferenceOutcome::Unsupported => totals.unsupported += 1,
            ReferenceOutcome::KnownBug => totals.known_bug += 1,
            ReferenceOutcome::Timeout => totals.timeout += 1,
            ReferenceOutcome::Error => totals.error += 1,
        }
    }
    totals
}

fn same_observation(left: &ReferenceCase, right: &ReferenceCase) -> bool {
    left.spec == right.spec
        && left.index == right.index
        && left.line == right.line
        && left.description == right.description
        && left.outcome == right.outcome
        && left.status == right.status
        && left.qualifier == right.qualifier
        && left.difference_fields == right.difference_fields
        && left.note == right.note
}

fn disposition(
    observation: &ReferenceCase,
    catalog: &CatalogCase,
) -> Result<(Disposition, RecordedOutcome)> {
    Ok(match observation.outcome {
        ReferenceOutcome::Pass => return Err("passing case cannot be excluded".into()),
        ReferenceOutcome::Fail if catalog.version_specific_bash => {
            (Disposition::VersionInapplicable, RecordedOutcome::Fail)
        }
        ReferenceOutcome::Fail => (Disposition::ReferenceFailure, RecordedOutcome::Fail),
        ReferenceOutcome::Unsupported => (Disposition::Unsupported, RecordedOutcome::Unsupported),
        ReferenceOutcome::KnownBug => (Disposition::KnownUpstreamBug, RecordedOutcome::KnownBug),
        ReferenceOutcome::Timeout => (Disposition::Timeout, RecordedOutcome::Timeout),
        ReferenceOutcome::Error => (Disposition::HarnessError, RecordedOutcome::Error),
    })
}

fn build_group_profiles(
    catalog: &[CatalogCase],
    cases: &CaseManifest,
    reports: &[ReferenceReport],
) -> Result<Vec<GroupProfile>> {
    let eligible = cases.eligible.iter().collect::<BTreeSet<_>>();
    let mut result = Vec::new();
    for group in GROUPS {
        let report = reports
            .iter()
            .find(|report| report.group == group)
            .ok_or_else(|| format!("missing {group} report"))?;
        let mut profile = GroupProfile {
            id: group.to_owned(),
            selected: catalog
                .iter()
                .filter(|case| case.groups.iter().any(|candidate| candidate == group))
                .count(),
            eligible: catalog
                .iter()
                .filter(|case| {
                    case.groups.iter().any(|candidate| candidate == group)
                        && eligible.contains(&case.id)
                })
                .count(),
            reference_failure: 0,
            unsupported: 0,
            known_upstream_bug: 0,
            timeout: 0,
            harness_error: 0,
            version_inapplicable: 0,
        };
        for case in cases
            .excluded
            .iter()
            .filter(|case| case.groups.iter().any(|candidate| candidate == group))
        {
            match case.disposition {
                Disposition::ReferenceFailure => profile.reference_failure += 1,
                Disposition::Unsupported => profile.unsupported += 1,
                Disposition::KnownUpstreamBug => profile.known_upstream_bug += 1,
                Disposition::Timeout => profile.timeout += 1,
                Disposition::HarnessError => profile.harness_error += 1,
                Disposition::VersionInapplicable => profile.version_inapplicable += 1,
            }
        }
        if profile.selected != report.totals.selected
            || profile.eligible != report.totals.pass
            || profile.reference_failure + profile.version_inapplicable != report.totals.fail
            || profile.unsupported != report.totals.unsupported
            || profile.known_upstream_bug != report.totals.known_bug
            || profile.timeout != report.totals.timeout
            || profile.harness_error != report.totals.error
        {
            return Err(format!("{group} disposition totals do not match its report").into());
        }
        result.push(profile);
    }
    Ok(result)
}

fn expected_source_profile() -> SourceProfile {
    SourceProfile {
        archive: SOURCE_FILE.to_owned(),
        url: SOURCE_URL.to_owned(),
        signature_url: SOURCE_SIGNATURE_URL.to_owned(),
        sha256: SOURCE_SHA256.to_owned(),
    }
}

fn expected_patch_profiles() -> Vec<PatchProfile> {
    PATCHES
        .iter()
        .enumerate()
        .map(|(index, (file, sha256))| PatchProfile {
            level: (index + 1) as u32,
            file: (*file).to_owned(),
            url: format!("https://ftp.gnu.org/gnu/bash/bash-5.3-patches/{file}"),
            sha256: (*sha256).to_owned(),
        })
        .collect()
}

fn expected_build_profile() -> BuildProfile {
    BuildProfile {
        system: BUILD_SYSTEM.to_owned(),
        prefix: BUILD_PREFIX.to_owned(),
        configure_args: configure_args(),
        cc: "/usr/bin/cc".to_owned(),
        cflags_template: CFLAGS_TEMPLATE.to_owned(),
        make_args: vec!["-s".to_owned(), "-j2".to_owned()],
        source_date_epoch: SOURCE_DATE_EPOCH,
        environment: BTreeMap::from([
            ("HOME".to_owned(), "{output}/home".to_owned()),
            ("LC_ALL".to_owned(), "C".to_owned()),
            ("PATH".to_owned(), FIXED_PATH.to_owned()),
            ("TZ".to_owned(), "UTC".to_owned()),
        ]),
        tools: pinned_tools(),
    }
}

fn expected_execution_profile(containment: String) -> ExecutionProfile {
    ExecutionProfile {
        kernel: KERNEL.to_owned(),
        containment,
        locale: "C.UTF-8".to_owned(),
        timezone: "UTC".to_owned(),
        base_path: FIXED_PATH.to_owned(),
        environment_is_cleared: true,
        environment: BTreeMap::from([
            ("LC_ALL".to_owned(), "C.UTF-8".to_owned()),
            ("LOCALE_ARCHIVE".to_owned(), "".to_owned()),
            ("OILS_GC_ON_EXIT".to_owned(), "".to_owned()),
            ("PATH".to_owned(), "{fixture-bin}:/usr/bin:/bin".to_owned()),
            ("REPO_ROOT".to_owned(), "{fixture-root}".to_owned()),
            ("SH".to_owned(), "{oracle-binary}".to_owned()),
            ("TMP".to_owned(), "{case-scratch}".to_owned()),
            ("TZ".to_owned(), "UTC".to_owned()),
        ]),
        unset: vec!["BASH_ENV".to_owned(), "ENV".to_owned(), "HOME".to_owned()],
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// [spec:nsh:req:compat.bash.reference-profile]
pub(crate) fn verify(
    root: &Path,
    sources: Option<&Path>,
    shell: Option<&Path>,
) -> Result<VerifiedReference> {
    let root = fs::canonicalize(root)?;
    let profile_path = root.join(PROFILE_FILE);
    let profile_text = fs::read_to_string(&profile_path)?;
    let profile: ReferenceProfile = toml::from_str(&profile_text)?;
    if profile_text != toml::to_string_pretty(&profile)? {
        return Err(format!("{} is not in canonical form", profile_path.display()).into());
    }
    validate_static_profile(&profile)?;

    let lock = crate::read_lock(&root)?;
    crate::verify_import(&root, &lock)?;
    crate::verify_oils_manifest(&root, &lock)?;
    let manifest_path = root.join("MANIFEST.toml");
    let manifest_text = fs::read_to_string(&manifest_path)?;
    let manifest: crate::OilsManifest = toml::from_str(&manifest_text)?;
    if profile.oils.commit != lock.commit
        || profile.oils.tree != lock.tree
        || profile.oils.spec_format != lock.spec_format
        || profile.oils.manifest_sha256 != sha256_bytes(manifest_text.as_bytes())
    {
        return Err("Bash profile does not identify the pinned Oils corpus".into());
    }

    let cases_path = root.join(&profile.calibration.case_manifest);
    let case_bytes = fs::read(&cases_path)?;
    if sha256_bytes(&case_bytes) != profile.calibration.case_manifest_sha256 {
        return Err(format!(
            "{} digest does not match the Bash profile",
            cases_path.display()
        )
        .into());
    }
    let case_text = std::str::from_utf8(&case_bytes)?;
    let cases: CaseManifest = serde_json::from_str(case_text)?;
    let canonical_cases = format!("{}\n", serde_json::to_string_pretty(&cases)?);
    if case_text != canonical_cases {
        return Err(format!("{} is not in canonical form", cases_path.display()).into());
    }
    if cases.schema != 1
        || cases.oracle_version != profile.version
        || cases.oracle_binary_sha256 != profile.calibration.binary_sha256
        || cases.oils_commit != profile.oils.commit
        || cases.oils_tree != profile.oils.tree
    {
        return Err("Bash case manifest identity does not match its profile".into());
    }
    let catalog = crate::oils_runner::bash_case_catalog(&root, &manifest)?;
    validate_case_manifest(&cases, &catalog)?;
    let groups = group_profiles_from_cases(&catalog, &cases)?;
    if groups != profile.calibration.groups
        || profile.calibration.eligible != cases.eligible.len()
        || profile.calibration.excluded != cases.excluded.len()
    {
        return Err("Bash profile disposition totals do not match its case manifest".into());
    }

    if let Some(sources) = sources {
        verify_source_files(sources)?;
    }
    if let Some(shell) = shell {
        let version = bash_version(shell)?;
        if version != profile.version {
            return Err(format!(
                "reference shell reports {version:?}, expected {:?}",
                profile.version
            )
            .into());
        }
        verify_digest(shell, &profile.calibration.binary_sha256)?;
    }
    Ok(VerifiedReference {
        version: profile.version,
        eligible: cases.eligible.len(),
        excluded: cases.excluded.len(),
    })
}

fn validate_static_profile(profile: &ReferenceProfile) -> Result<()> {
    if profile.schema != 1
        || profile.name != "GNU Bash 5.3 differential reference"
        || profile.release != RELEASE
        || profile.patch_level != PATCH_LEVEL
        || profile.version != ORACLE_VERSION
        || profile.source != expected_source_profile()
        || profile.patches != expected_patch_profiles()
        || profile.build != expected_build_profile()
    {
        return Err("Bash reference source or build identity is not the pinned profile".into());
    }
    let expected_execution = expected_execution_profile("sandbox-pid-net-ro-root".to_owned());
    if profile.execution != expected_execution {
        return Err("Bash reference execution environment is not the pinned profile".into());
    }
    if profile.oils.manifest != "MANIFEST.toml"
        || profile.oils.expectation_shell != "bash"
        || profile.oils.groups != GROUPS.into_iter().map(str::to_owned).collect::<Vec<_>>()
        || profile.calibration.date != "2026-08-19"
        || profile.calibration.timeout_ms != 5_000
        || profile.calibration.case_manifest != CASES_FILE
        || !valid_sha256(&profile.calibration.binary_sha256)
        || !valid_sha256(&profile.calibration.case_manifest_sha256)
    {
        return Err("Bash reference calibration metadata is invalid".into());
    }
    Ok(())
}

fn validate_case_manifest(cases: &CaseManifest, catalog: &[CatalogCase]) -> Result<()> {
    if !strictly_sorted(&cases.eligible) {
        return Err("eligible Bash case IDs are not strictly sorted".into());
    }
    if !cases
        .excluded
        .windows(2)
        .all(|pair| pair[0].id < pair[1].id)
    {
        return Err("excluded Bash cases are not strictly sorted".into());
    }
    let catalog_by_id = unique_catalog(catalog)?;
    let mut accounted = BTreeSet::new();
    for id in &cases.eligible {
        if !catalog_by_id.contains_key(id) {
            return Err(format!("eligible case {id} is absent from the Oils manifest").into());
        }
        accounted.insert(id.as_str());
    }
    for excluded in &cases.excluded {
        let catalog_case = catalog_by_id.get(&excluded.id).ok_or_else(|| {
            format!(
                "excluded case {} is absent from the Oils manifest",
                excluded.id
            )
        })?;
        if excluded.spec != catalog_case.spec
            || excluded.index != catalog_case.index
            || excluded.line != catalog_case.line
            || excluded.description != catalog_case.description
            || excluded.groups != catalog_case.groups
        {
            return Err(format!("excluded case {} has stale corpus metadata", excluded.id).into());
        }
        validate_disposition(excluded, catalog_case)?;
        if !accounted.insert(&excluded.id) {
            return Err(format!("case {} has more than one disposition", excluded.id).into());
        }
    }
    let expected = catalog_by_id
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if accounted != expected {
        return Err("Bash case manifest does not account for the exact comparison set".into());
    }
    Ok(())
}

fn validate_disposition(case: &ExcludedCase, catalog: &CatalogCase) -> Result<()> {
    let valid = match (case.reference_outcome, case.disposition) {
        (RecordedOutcome::Fail, Disposition::VersionInapplicable) => catalog.version_specific_bash,
        (RecordedOutcome::Fail, Disposition::ReferenceFailure) => !catalog.version_specific_bash,
        (RecordedOutcome::Unsupported, Disposition::Unsupported)
        | (RecordedOutcome::KnownBug, Disposition::KnownUpstreamBug)
        | (RecordedOutcome::Timeout, Disposition::Timeout)
        | (RecordedOutcome::Error, Disposition::HarnessError) => true,
        _ => false,
    };
    if !valid {
        return Err(format!("case {} has an invalid disposition", case.id).into());
    }
    if matches!(case.reference_outcome, RecordedOutcome::Fail) && case.difference_fields.is_empty()
    {
        return Err(format!("failed case {} records no differing field", case.id).into());
    }
    if matches!(
        case.reference_outcome,
        RecordedOutcome::Unsupported | RecordedOutcome::KnownBug
    ) && !case.difference_fields.is_empty()
    {
        return Err(format!("qualified case {} unexpectedly records a mismatch", case.id).into());
    }
    Ok(())
}

fn group_profiles_from_cases(
    catalog: &[CatalogCase],
    cases: &CaseManifest,
) -> Result<Vec<GroupProfile>> {
    let eligible = cases.eligible.iter().collect::<BTreeSet<_>>();
    let mut result = Vec::new();
    for group in GROUPS {
        let mut profile = GroupProfile {
            id: group.to_owned(),
            selected: 0,
            eligible: 0,
            reference_failure: 0,
            unsupported: 0,
            known_upstream_bug: 0,
            timeout: 0,
            harness_error: 0,
            version_inapplicable: 0,
        };
        for case in catalog
            .iter()
            .filter(|case| case.groups.iter().any(|candidate| candidate == group))
        {
            profile.selected += 1;
            if eligible.contains(&case.id) {
                profile.eligible += 1;
            }
        }
        for case in cases
            .excluded
            .iter()
            .filter(|case| case.groups.iter().any(|candidate| candidate == group))
        {
            match case.disposition {
                Disposition::ReferenceFailure => profile.reference_failure += 1,
                Disposition::Unsupported => profile.unsupported += 1,
                Disposition::KnownUpstreamBug => profile.known_upstream_bug += 1,
                Disposition::Timeout => profile.timeout += 1,
                Disposition::HarnessError => profile.harness_error += 1,
                Disposition::VersionInapplicable => profile.version_inapplicable += 1,
            }
        }
        let accounted = profile.eligible
            + profile.reference_failure
            + profile.unsupported
            + profile.known_upstream_bug
            + profile.timeout
            + profile.harness_error
            + profile.version_inapplicable;
        if accounted != profile.selected {
            return Err(format!(
                "{group} profile accounts for {accounted}/{} cases",
                profile.selected
            )
            .into());
        }
        result.push(profile);
    }
    Ok(result)
}

fn strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests;
