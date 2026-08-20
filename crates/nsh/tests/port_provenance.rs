use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nsh lives below the repository's crates directory")
        .to_owned()
}

fn files_below(root: &Path, extension: &str) -> Vec<PathBuf> {
    fn visit(directory: &Path, extension: &str, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).expect("source directory is readable") {
            let path = entry.expect("source entry is readable").path();
            if path.is_dir() {
                visit(&path, extension, files);
            } else if path.extension().is_some_and(|value| value == extension) {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(root, extension, &mut files);
    files.sort();
    files
}

fn dash_semantics(text: &str, definitions_only: bool) -> BTreeSet<String> {
    const PREFIX: &str = "[spec:dash:sem:";
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        let line = if definitions_only {
            let Some(line) = line.trim_start().strip_prefix("> ") else {
                continue;
            };
            line
        } else {
            line
        };
        let mut remaining = line;
        while let Some(start) = remaining.find(PREFIX) {
            let after_prefix = start + PREFIX.len();
            let literal = &remaining[after_prefix..];
            let Some(end) = literal.find(']') else {
                break;
            };
            let literal = &literal[..end];
            if !literal.contains("/test") {
                let id = literal.split('/').next().unwrap_or(literal);
                let id = id.split('+').next().unwrap_or(id);
                ids.insert(id.to_owned());
            }
            remaining = &remaining[after_prefix + end + 1..];
        }
    }
    ids
}

fn assignment<'a>(lock: &'a str, name: &str) -> &'a str {
    let prefix = format!("{name}='");
    lock.lines()
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or_else(|| panic!("missing {name} in reference lock"))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// [spec:nsh:req:idiom.port-provenance+1/test]
#[test]
fn port_topology_is_not_indexed() {
    let root = repository_root();
    let structural_claim = format!("[spec:dash:{}:", "def");
    for path in files_below(&root.join("crates"), "rs")
        .into_iter()
        .chain(files_below(&root.join("docs/spec/port"), "md"))
    {
        let text = fs::read_to_string(&path).expect("provenance source is UTF-8");
        assert!(
            !text.contains(&structural_claim),
            "obsolete Dash topology claim in {}",
            path.display()
        );
    }

    assert!(!root.join("plan/.port-manifest.styx").exists());
    assert!(!root.join("plan/annotations.styx").exists());

    let nspec = fs::read_to_string(root.join(".config/nspec/config.styx")).unwrap();
    assert!(!nspec.contains("source-impl"));
    assert!(!nspec.contains("src/**/*.c"));
    assert!(!nspec.contains("src/**/*.h"));

    let nplan = fs::read_to_string(root.join(".config/nplan/config.styx")).unwrap();
    assert!(!nplan.contains("scope \"porting\""));
    assert!(!nplan.contains("mode @both"));
}

// [spec:nsh:req:idiom.port-provenance+1/test]
#[test]
fn behavioral_rules_stay_linked() {
    let root = repository_root();
    let mut rules = BTreeSet::new();
    for path in files_below(&root.join("docs/spec/port"), "md") {
        let text = fs::read_to_string(path).expect("Dash spec source is UTF-8");
        rules.extend(dash_semantics(&text, true));
    }

    let mut implementations = BTreeSet::new();
    for path in files_below(&root.join("crates"), "rs") {
        let text = fs::read_to_string(path).expect("Rust source is UTF-8");
        implementations.extend(dash_semantics(&text, false));
    }

    let missing: Vec<_> = rules.difference(&implementations).cloned().collect();
    assert!(missing.is_empty(), "unlinked Dash behavior: {missing:?}");
}

// [spec:nsh:req:idiom.port-provenance+1/test]
#[test]
fn reference_lock_is_complete() {
    let root = repository_root();
    let lock = fs::read_to_string(root.join("tests/DASH_REFERENCE.env")).unwrap();
    assert!(assignment(&lock, "DASH_REFERENCE_URL").starts_with("https://"));
    assert!(!assignment(&lock, "DASH_REFERENCE_TAG").is_empty());
    assert!(is_lower_hex(assignment(&lock, "DASH_REFERENCE_COMMIT"), 40));
    for name in [
        "DASH_REFERENCE_ARCHIVE_SHA256",
        "DASH_REFERENCE_PATCH_1_SHA256",
        "DASH_REFERENCE_PATCH_2_SHA256",
    ] {
        assert!(is_lower_hex(assignment(&lock, name), 64), "invalid {name}");
    }

    let script = fs::read_to_string(root.join("tests/build-reference.sh")).unwrap();
    let containment = script.find("scripts/sandboxed").unwrap();
    let extraction = script.find("tar -xzf").unwrap();
    assert!(containment < extraction);
    assert!(script.contains("sha256sum -c -"));
    assert!(script.contains("verify_patch"));
    assert!(!script.contains("tar -cf"));
}
