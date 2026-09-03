use super::spec::parse_spec_bytes;
use super::*;

/// The system shell the runner cases drive their subject through.
///
/// Not optional. Skipping when `/bin/sh` is absent made four checks of
/// the runner's containment, isolation and verdict reporting pass on a
/// host where none of them had run
/// ([`spec:nsh:req:oracle.cannot-measure-is-a-failure`]).
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
fn system_shell() -> &'static Path {
    let shell = Path::new("/bin/sh");
    assert!(
        shell.exists(),
        "/bin/sh is required by this test: there is no subject to run the \
         case under without it"
    );
    shell
}

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

/// Every group the manifest defines is one `selecting_dialect` has been
/// told about.
///
/// Its unknown arm answers `None`, which is right for a group no dialect
/// decides and silently wrong for a sixth Bash group. Pinning the list
/// is what makes a new group cost that decision rather than default past
/// it.
// [spec:nsh:req:oracle.refuses-a-mismeasurement/test]
#[test]
fn every_group_names_its_dialect() {
    let manifest: crate::OilsManifest =
        toml::from_str(&fs::read_to_string(crate::survey_root().join("MANIFEST.toml")).unwrap())
            .unwrap();
    let defined: BTreeSet<&str> = manifest
        .groups
        .iter()
        .map(|group| group.id.as_str())
        .collect();
    assert_eq!(
        defined,
        BTreeSet::from([
            "full",
            "posix-candidate",
            "bash-comparison",
            "bash-extension",
            "bash-named-diagnostic",
        ]),
        "a group was added or renamed without answering for its dialect",
    );
}

/// A Bash-selected group refuses to be scored in the POSIX dialect, and
/// says which of the two configurations it refused on.
///
/// The measurement this exists for is in `run_manifest`'s comment: the
/// same 60 cases fail 22 times under the default expectation and 7 times
/// under `bash`, with no third thing in the output to tell them apart.
// [spec:nsh:req:oracle.refuses-a-mismeasurement/test]
#[test]
fn a_bash_group_refuses_posix_scoring() {
    let shell = Path::new("target/release/nsh");
    for group in ["bash-comparison", "bash-extension", "bash-named-diagnostic"] {
        let complaint = refuse_a_dialect_mismatch(group, "osh", shell)
            .expect_err("a Bash group was scored in the POSIX dialect")
            .to_string();
        assert!(complaint.contains(group), "{complaint}");
        assert!(complaint.contains("--expect-shell bash"), "{complaint}");
        assert!(complaint.contains("\"osh\""), "{complaint}");
        assert!(complaint.contains("\"nsh\""), "{complaint}");

        refuse_a_dialect_mismatch(group, "bash", shell)
            .expect("a Bash group was refused its own dialect");
        /* The manifest spells some files' expectations `bash-4.4`, and
         * `expectation_key` folds the version token away, so a run naming
         * one is asking for the same namespace and not a third dialect. */
        refuse_a_dialect_mismatch(group, "bash-4.4", shell)
            .expect("a version-qualified Bash namespace was read as another dialect");
    }
}

/// A group no dialect decides is scored in whichever the caller asked
/// for.
///
/// `posix-candidate` is the one that matters: its selector is dash, whose
/// dialect is the one nsh runs under every name but `bash`, so the
/// documented `run-oils --group posix-candidate` -- which takes the
/// default `osh` namespace -- is already asking its own question.
// [spec:nsh:req:oracle.refuses-a-mismeasurement/test]
#[test]
fn an_undecided_group_is_left_alone() {
    let shell = Path::new("target/release/nsh");
    for group in ["full", "posix-candidate"] {
        for expectation in ["osh", "bash", "dash"] {
            refuse_a_dialect_mismatch(group, expectation, shell)
                .expect("a group no dialect decides was refused a namespace");
        }
    }
}

#[test]
fn process_timeout_kills_background_descendants() {
    let shell = system_shell();
    let scratch = ScratchTree::new().unwrap();
    let containment = Containment::verified(scratch.path()).unwrap();
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
        containment: &containment,
        timezone: None,
        locale_archive: None,
    };
    let process = run_process(
        &context,
        &case_dir,
        b"(sleep 1; printf leaked > \"$TMP/leak\") & wait\n",
    )
    .unwrap();
    assert!(process.timed_out);
    std::thread::sleep(Duration::from_millis(1_100));
    assert!(!case_dir.join("leak").exists());
}

#[test]
fn process_gets_isolated_env_and_cwd() {
    let shell = system_shell();
    let scratch = ScratchTree::new().unwrap();
    let containment = Containment::verified(scratch.path()).unwrap();
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
        containment: &containment,
        timezone: None,
        locale_archive: None,
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
    let shell = system_shell();
    let parsed = parse_spec_bytes(
        b"#### qualified\nprintf 'ok\\n'; printf 'err\\n' >&2; exit 3\n\
          ## stdout-json: \"ok\\n\"\n## stderr: err\n## status: 3\n\
          ## N-I sh stdout-json: \"ok\\n\"\n",
    )
    .unwrap();
    let scratch = ScratchTree::new().unwrap();
    let containment = Containment::verified(scratch.path()).unwrap();
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
        containment: &containment,
        timezone: None,
        locale_archive: None,
    };
    let process = run_process(&context, &case_dir, &parsed.cases[0].code).unwrap();
    let record = evaluate_case(&context, "fixture.test.sh", 0, &parsed.cases[0], process);
    assert_eq!(record.outcome, Outcome::Unsupported);
    assert_eq!(record.qualifier.as_deref(), Some("N-I"));
    assert!(record.differences.is_empty());
}

#[test]
fn evaluation_reports_byte_exact_mismatch() {
    let shell = system_shell();
    let parsed = parse_spec_bytes(b"#### mismatch\nprintf actual\n## stdout: expected\n").unwrap();
    let scratch = ScratchTree::new().unwrap();
    let containment = Containment::verified(scratch.path()).unwrap();
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
        containment: &containment,
        timezone: None,
        locale_archive: None,
    };
    let process = run_process(&context, &case_dir, &parsed.cases[0].code).unwrap();
    let record = evaluate_case(&context, "fixture.test.sh", 0, &parsed.cases[0], process);
    assert_eq!(record.outcome, Outcome::Fail);
    assert_eq!(record.differences.len(), 1);
    assert_eq!(record.differences[0].field, "stdout");
}

#[test]
fn summary_omits_timing_and_lists_failures() {
    let scratch = ScratchTree::new().unwrap();
    let path = scratch.path().join("result.toml");
    let report = RunReport {
        schema: 1,
        survey: "oils-shell-spec",
        source_commit: "0123456789abcdef".to_owned(),
        group: "bash-extension".to_owned(),
        group_label: "Bash extension survey".to_owned(),
        shell: "target/release/nsh".to_owned(),
        shell_sha256: "abcdef".to_owned(),
        expectation_shell: "bash".to_owned(),
        containment: "sandbox-pid-net-ro-root".to_owned(),
        posix: false,
        timeout_ms: 5_000,
        elapsed_ms: 999,
        totals: Totals {
            selected: 2,
            executed: 2,
            pass: 1,
            fail: 1,
            ..Totals::default()
        },
        cases: vec![
            CaseRecord {
                spec: "sample.test.sh".to_owned(),
                index: 0,
                line: 7,
                description: "passes".to_owned(),
                outcome: Outcome::Pass,
                status: Some(0),
                duration_ms: 101,
                qualifier: None,
                differences: Vec::new(),
                note: None,
            },
            CaseRecord {
                spec: "sample.test.sh".to_owned(),
                index: 1,
                line: 12,
                description: "fails".to_owned(),
                outcome: Outcome::Fail,
                status: Some(1),
                duration_ms: 202,
                qualifier: Some("BUG bash".to_owned()),
                differences: vec![Difference::integer("status", 0, 1)],
                note: None,
            },
        ],
    };

    write_summary(&path, &report).unwrap();
    let text = fs::read_to_string(path).unwrap();
    let summary: toml::Value = toml::from_str(&text).unwrap();
    let nonpassing = summary["nonpassing"].as_array().unwrap();
    assert_eq!(nonpassing.len(), 1);
    assert_eq!(nonpassing[0]["index"].as_integer(), Some(1));
    assert_eq!(
        nonpassing[0]["difference_fields"][0].as_str(),
        Some("status")
    );
    assert!(summary.get("elapsed_ms").is_none());
    assert!(!text.contains("duration_ms"));
}

#[test]
// [spec:nsh:req:idiom.conformance-closure/test]
fn recorded_bash_summaries_are_complete() {
    let root = crate::survey_root();
    let manifest: crate::OilsManifest =
        toml::from_str(&fs::read_to_string(root.join("MANIFEST.toml")).unwrap()).unwrap();
    for group_id in ["bash-comparison", "bash-extension", "bash-named-diagnostic"] {
        let path = root.join("results").join(format!("{group_id}.toml"));
        let summary: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let group = manifest
            .groups
            .iter()
            .find(|group| group.id == group_id)
            .unwrap();
        let totals = summary["totals"].as_table().unwrap();
        let selected = usize::try_from(totals["selected"].as_integer().unwrap()).unwrap();
        let executed = usize::try_from(totals["executed"].as_integer().unwrap()).unwrap();
        let pass = usize::try_from(totals["pass"].as_integer().unwrap()).unwrap();
        let nonpassing = summary["nonpassing"].as_array().unwrap();

        assert_eq!(
            summary["source_commit"].as_str(),
            Some(manifest.source_commit.as_str())
        );
        assert_eq!(summary["group"].as_str(), Some(group_id));
        assert_eq!(summary["expectation_shell"].as_str(), Some("bash"));
        assert_eq!(
            summary["containment"].as_str(),
            Some("sandbox-pid-net-ro-root")
        );
        assert_eq!(selected, group.cases);
        assert_eq!(executed, selected);
        assert_eq!(totals["skip"].as_integer(), Some(0));
        assert_eq!(totals["timeout"].as_integer(), Some(0));
        assert_eq!(totals["error"].as_integer(), Some(0));
        assert_eq!(nonpassing.len(), selected - pass);
        assert_eq!(summary["shell_sha256"].as_str().unwrap().len(), 64);
    }
}
