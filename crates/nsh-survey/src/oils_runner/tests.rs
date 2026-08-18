use super::*;

#[test]
fn parses_json_multiline_and_qualifiers() {
    let parsed = parse_spec_bytes(
        br#"## compare_shells: dash bash

#### bytes and status
printf ignored
## stdout-json: "\u03bb\n"
## status: 3
## BUG osh STDOUT:
known
## END
## BUG osh status: 0
"#,
    )
    .unwrap();
    assert_eq!(parsed.cases.len(), 1);
    let case = &parsed.cases[0];
    assert_eq!(case.ideal.stdout[0].bytes, "λ\n".as_bytes());
    assert_eq!(case.ideal.status, Some(3));
    let osh = case.per_shell.get("osh").unwrap();
    assert_eq!(osh.qualifier, "BUG");
    assert_eq!(osh.assertions.stdout[0].bytes, b"known\n");
    assert_eq!(osh.assertions.status, Some(0));
}

#[test]
fn comments_are_ignored_in_raw_sections() {
    let parsed = parse_spec_bytes(
        b"#### comments\n# ignored\nprintf x\n## STDOUT:\n# ignored too\nx\n## END\n",
    )
    .unwrap();
    assert_eq!(parsed.cases[0].code, b"printf x\n");
    assert_eq!(parsed.cases[0].ideal.stdout[0].bytes, b"x\n");
}

#[test]
fn rejects_qualified_metadata_conflicts() {
    let duplicate = b"#### duplicate\nprintf x\n## BUG osh stdout: x\n## BUG osh status: 1\n## BUG osh stderr: x\n## BUG osh stdout-json: \"x\\n\"\n";
    assert!(
        parse_spec_bytes(duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate stdout")
    );
    let mixed = b"#### mixed\nprintf x\n## BUG osh stdout: x\n## OK osh status: 0\n";
    assert!(
        parse_spec_bytes(mixed)
            .unwrap_err()
            .to_string()
            .contains("inconsistent qualifier")
    );
}

#[test]
fn capture_discards_bytes_past_the_bound() {
    let input = vec![b'x'; OUTPUT_LIMIT + 17];
    let captured = capture(std::io::Cursor::new(input)).unwrap();
    assert!(captured.truncated);
    assert_eq!(captured.bytes.len(), OUTPUT_LIMIT);
}

#[test]
fn parses_every_imported_oils_case() {
    let root = crate::survey_root();
    let manifest: crate::OilsManifest =
        toml::from_str(&fs::read_to_string(root.join("MANIFEST.toml")).unwrap()).unwrap();
    let mut cases = 0;
    for spec in manifest.specs {
        let parsed = parse_spec(&root.join(&spec.path)).unwrap();
        assert_eq!(parsed.cases.len(), spec.cases, "{}", spec.path);
        cases += parsed.cases.len();
    }
    assert_eq!(cases, 2755);
}

#[test]
fn process_timeout_kills_background_descendants() {
    let shell = Path::new("/bin/sh");
    if !shell.exists() {
        return;
    }
    let scratch = ScratchTree::new().unwrap();
    let case_dir = scratch.path().join("case");
    fs::create_dir(&case_dir).unwrap();
    let path = env::var_os("PATH").unwrap_or_default();
    let context = RunContext {
        root: scratch.path(),
        shell,
        expectation_shell: "sh",
        timeout: Duration::from_millis(50),
        posix: false,
        survey_path: path,
        scratch: scratch.path(),
    };
    let process = run_process(
        &context,
        &case_dir,
        b"(sleep 1; printf leaked > \"$TMP/leak\") & wait\n",
    )
    .unwrap();
    assert!(process.timed_out);
    thread::sleep(Duration::from_millis(1_100));
    assert!(!case_dir.join("leak").exists());
}

#[test]
fn process_gets_isolated_env_and_cwd() {
    let shell = Path::new("/bin/sh");
    if !shell.exists() {
        return;
    }
    let scratch = ScratchTree::new().unwrap();
    let case_dir = scratch.path().join("case");
    fs::create_dir(&case_dir).unwrap();
    let context = RunContext {
        root: scratch.path(),
        shell,
        expectation_shell: "sh",
        timeout: Duration::from_secs(2),
        posix: false,
        survey_path: env::var_os("PATH").unwrap_or_default(),
        scratch: scratch.path(),
    };
    let process = run_process(
        &context,
        &case_dir,
        b"test \"$PWD\" = \"$TMP\" && test -z \"${HOME+x}\" && printf isolated\n",
    )
    .unwrap();
    assert!(!process.timed_out);
    assert_eq!(process.status.code(), Some(0));
    assert_eq!(process.stdout.bytes, b"isolated");
    assert!(process.stderr.bytes.is_empty());
}

#[test]
fn evaluation_applies_json_status_and_qualifier() {
    let shell = Path::new("/bin/sh");
    if !shell.exists() {
        return;
    }
    let parsed = parse_spec_bytes(
        b"#### qualified\nprintf 'ok\\n'; printf 'err\\n' >&2; exit 3\n\
          ## stdout-json: \"ok\\n\"\n## stderr: err\n## status: 3\n\
          ## N-I sh stdout-json: \"ok\\n\"\n",
    )
    .unwrap();
    let scratch = ScratchTree::new().unwrap();
    let case_dir = scratch.path().join("case");
    fs::create_dir(&case_dir).unwrap();
    let context = RunContext {
        root: scratch.path(),
        shell,
        expectation_shell: "sh",
        timeout: Duration::from_secs(2),
        posix: false,
        survey_path: env::var_os("PATH").unwrap_or_default(),
        scratch: scratch.path(),
    };
    let process = run_process(&context, &case_dir, &parsed.cases[0].code).unwrap();
    let record = evaluate_case(&context, "fixture.test.sh", 0, &parsed.cases[0], process);
    assert_eq!(record.outcome, Outcome::Unsupported);
    assert_eq!(record.qualifier.as_deref(), Some("N-I"));
    assert!(record.differences.is_empty());
}

#[test]
fn evaluation_reports_byte_exact_mismatch() {
    let shell = Path::new("/bin/sh");
    if !shell.exists() {
        return;
    }
    let parsed = parse_spec_bytes(b"#### mismatch\nprintf actual\n## stdout: expected\n").unwrap();
    let scratch = ScratchTree::new().unwrap();
    let case_dir = scratch.path().join("case");
    fs::create_dir(&case_dir).unwrap();
    let context = RunContext {
        root: scratch.path(),
        shell,
        expectation_shell: "sh",
        timeout: Duration::from_secs(2),
        posix: false,
        survey_path: env::var_os("PATH").unwrap_or_default(),
        scratch: scratch.path(),
    };
    let process = run_process(&context, &case_dir, &parsed.cases[0].code).unwrap();
    let record = evaluate_case(&context, "fixture.test.sh", 0, &parsed.cases[0], process);
    assert_eq!(record.outcome, Outcome::Fail);
    assert_eq!(record.differences.len(), 1);
    assert_eq!(record.differences[0].field, "stdout");
}
