use super::*;

fn catalog_case(id: &str, groups: &[&str], version_specific_bash: bool) -> CatalogCase {
    let (spec, index) = id.split_once(':').unwrap();
    CatalogCase {
        id: id.to_owned(),
        spec: spec.to_owned(),
        index: index.parse().unwrap(),
        line: 7,
        description: format!("description for {id}"),
        groups: groups.iter().map(|group| (*group).to_owned()).collect(),
        version_specific_bash,
    }
}

fn reference_case(id: &str, outcome: ReferenceOutcome) -> ReferenceCase {
    let catalog = catalog_case(id, &["bash-comparison"], false);
    ReferenceCase {
        id: catalog.id,
        spec: catalog.spec,
        index: catalog.index,
        line: catalog.line,
        description: catalog.description,
        outcome,
        status: Some(0),
        qualifier: None,
        difference_fields: if outcome == ReferenceOutcome::Fail {
            vec!["stdout".to_owned()]
        } else {
            vec![]
        },
        note: None,
    }
}

fn excluded_case(
    catalog: &CatalogCase,
    disposition: Disposition,
    reference_outcome: RecordedOutcome,
) -> ExcludedCase {
    ExcludedCase {
        id: catalog.id.clone(),
        spec: catalog.spec.clone(),
        index: catalog.index,
        line: catalog.line,
        description: catalog.description.clone(),
        groups: catalog.groups.clone(),
        disposition,
        reference_outcome,
        status: Some(0),
        qualifier: None,
        difference_fields: if reference_outcome == RecordedOutcome::Fail {
            vec!["stdout".to_owned()]
        } else {
            vec![]
        },
        note: None,
    }
}

fn report(group: &str, cases: Vec<ReferenceCase>) -> ReferenceReport {
    ReferenceReport {
        source_commit: "commit".to_owned(),
        group: group.to_owned(),
        shell_sha256: "a".repeat(64),
        containment: "sandbox-pid-net-ro-root".to_owned(),
        timeout_ms: 5_000,
        totals: observed_totals(&cases),
        cases,
    }
}

#[test]
fn source_pin_is_complete_and_contiguous() {
    assert_eq!(PATCHES.len(), PATCH_LEVEL as usize);
    for (index, (file, digest)) in PATCHES.iter().enumerate() {
        assert_eq!(*file, format!("bash53-{:03}", index + 1));
        assert!(valid_sha256(digest));
    }
    assert!(valid_sha256(SOURCE_SHA256));
    assert_eq!(expected_patch_profiles().last().unwrap().level, 15);
}

#[test]
fn sha256_requires_lowercase_hex() {
    assert!(valid_sha256(&"0".repeat(64)));
    assert!(valid_sha256(&"abcdef0123456789".repeat(4)));
    assert!(!valid_sha256(&"0".repeat(63)));
    assert!(!valid_sha256(&"0".repeat(65)));
    assert!(!valid_sha256(&"G".repeat(64)));
    assert!(!valid_sha256(&"A".repeat(64)));
}

#[test]
fn byte_digest_matches_the_standard_vector() {
    assert_eq!(
        sha256_bytes(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn strict_sorting_rejects_bad_order() {
    assert!(strictly_sorted(&[]));
    assert!(strictly_sorted(&["a".to_owned()]));
    assert!(strictly_sorted(&["a".to_owned(), "b".to_owned()]));
    assert!(!strictly_sorted(&["a".to_owned(), "a".to_owned()]));
    assert!(!strictly_sorted(&["b".to_owned(), "a".to_owned()]));
}

#[test]
fn nonpass_outcomes_map_to_dispositions() {
    let normal = catalog_case("x.test.sh:0", &["bash-comparison"], false);
    let versioned = catalog_case("x.test.sh:0", &["bash-comparison"], true);
    let cases = [
        (ReferenceOutcome::Fail, Disposition::ReferenceFailure),
        (ReferenceOutcome::Unsupported, Disposition::Unsupported),
        (ReferenceOutcome::KnownBug, Disposition::KnownUpstreamBug),
        (ReferenceOutcome::Timeout, Disposition::Timeout),
        (ReferenceOutcome::Error, Disposition::HarnessError),
    ];
    for (outcome, expected) in cases {
        assert_eq!(
            disposition(&reference_case("x.test.sh:0", outcome), &normal)
                .unwrap()
                .0,
            expected
        );
    }
    assert_eq!(
        disposition(
            &reference_case("x.test.sh:0", ReferenceOutcome::Fail),
            &versioned
        )
        .unwrap()
        .0,
        Disposition::VersionInapplicable
    );
    assert!(
        disposition(
            &reference_case("x.test.sh:0", ReferenceOutcome::Pass),
            &normal
        )
        .is_err()
    );
}

#[test]
fn disposition_validation_rejects_category_drift() {
    let normal = catalog_case("x.test.sh:0", &["bash-comparison"], false);
    let versioned = catalog_case("x.test.sh:0", &["bash-comparison"], true);
    let good = excluded_case(
        &normal,
        Disposition::ReferenceFailure,
        RecordedOutcome::Fail,
    );
    assert!(validate_disposition(&good, &normal).is_ok());

    let wrong_version = ExcludedCase {
        disposition: Disposition::VersionInapplicable,
        ..good.clone()
    };
    assert!(validate_disposition(&wrong_version, &normal).is_err());
    assert!(validate_disposition(&wrong_version, &versioned).is_ok());

    let wrong_outcome = ExcludedCase {
        reference_outcome: RecordedOutcome::KnownBug,
        ..good.clone()
    };
    assert!(validate_disposition(&wrong_outcome, &normal).is_err());

    let no_difference = ExcludedCase {
        difference_fields: vec![],
        ..good
    };
    assert!(validate_disposition(&no_difference, &normal).is_err());
}

#[test]
fn qualified_dispositions_cannot_hide_a_mismatch() {
    let catalog = catalog_case("x.test.sh:0", &["bash-comparison"], false);
    let mut unsupported = excluded_case(
        &catalog,
        Disposition::Unsupported,
        RecordedOutcome::Unsupported,
    );
    assert!(validate_disposition(&unsupported, &catalog).is_ok());
    unsupported.difference_fields.push("status".to_owned());
    assert!(validate_disposition(&unsupported, &catalog).is_err());

    let mut known_bug = excluded_case(
        &catalog,
        Disposition::KnownUpstreamBug,
        RecordedOutcome::KnownBug,
    );
    assert!(validate_disposition(&known_bug, &catalog).is_ok());
    known_bug.difference_fields.push("stderr".to_owned());
    assert!(validate_disposition(&known_bug, &catalog).is_err());
}

#[test]
fn exact_accounting_accepts_single_disposition() {
    let pass = catalog_case("a.test.sh:0", &["bash-comparison"], false);
    let fail = catalog_case("b.test.sh:0", &["bash-comparison"], false);
    let cases = CaseManifest {
        schema: 1,
        oracle_version: ORACLE_VERSION.to_owned(),
        oracle_binary_sha256: "a".repeat(64),
        oils_commit: "commit".to_owned(),
        oils_tree: "tree".to_owned(),
        eligible: vec![pass.id.clone()],
        excluded: vec![excluded_case(
            &fail,
            Disposition::ReferenceFailure,
            RecordedOutcome::Fail,
        )],
    };
    assert!(validate_case_manifest(&cases, &[pass, fail]).is_ok());
}

#[test]
fn exact_accounting_rejects_bad_ids() {
    let catalog = catalog_case("a.test.sh:0", &["bash-comparison"], false);
    let base = CaseManifest {
        schema: 1,
        oracle_version: ORACLE_VERSION.to_owned(),
        oracle_binary_sha256: "a".repeat(64),
        oils_commit: "commit".to_owned(),
        oils_tree: "tree".to_owned(),
        eligible: vec![catalog.id.clone()],
        excluded: vec![],
    };
    assert!(validate_case_manifest(&base, std::slice::from_ref(&catalog)).is_ok());

    let missing = CaseManifest {
        eligible: vec![],
        ..base
    };
    assert!(validate_case_manifest(&missing, std::slice::from_ref(&catalog)).is_err());

    let duplicate = CaseManifest {
        eligible: vec![catalog.id.clone(), catalog.id.clone()],
        ..missing
    };
    assert!(validate_case_manifest(&duplicate, std::slice::from_ref(&catalog)).is_err());

    let unknown = CaseManifest {
        eligible: vec!["unknown.test.sh:0".to_owned()],
        ..duplicate
    };
    assert!(validate_case_manifest(&unknown, &[catalog]).is_err());
}

#[test]
fn exact_case_accounting_rejects_stale_metadata() {
    let catalog = catalog_case("a.test.sh:0", &["bash-comparison"], false);
    let mut excluded = excluded_case(
        &catalog,
        Disposition::ReferenceFailure,
        RecordedOutcome::Fail,
    );
    excluded.line += 1;
    let cases = CaseManifest {
        schema: 1,
        oracle_version: ORACLE_VERSION.to_owned(),
        oracle_binary_sha256: "a".repeat(64),
        oils_commit: "commit".to_owned(),
        oils_tree: "tree".to_owned(),
        eligible: vec![],
        excluded: vec![excluded],
    };
    assert!(validate_case_manifest(&cases, &[catalog]).is_err());
}

#[test]
fn observed_totals_classify_every_outcome() {
    let outcomes = [
        ReferenceOutcome::Pass,
        ReferenceOutcome::Fail,
        ReferenceOutcome::Unsupported,
        ReferenceOutcome::KnownBug,
        ReferenceOutcome::Timeout,
        ReferenceOutcome::Error,
    ];
    let cases = outcomes
        .into_iter()
        .enumerate()
        .map(|(index, outcome)| reference_case(&format!("x.test.sh:{index}"), outcome))
        .collect::<Vec<_>>();
    let totals = observed_totals(&cases);
    assert_eq!(totals.selected, 6);
    assert_eq!(totals.executed, 6);
    assert_eq!(totals.pass, 1);
    assert_eq!(totals.fail, 1);
    assert_eq!(totals.unsupported, 1);
    assert_eq!(totals.known_bug, 1);
    assert_eq!(totals.timeout, 1);
    assert_eq!(totals.error, 1);
}

#[test]
fn report_validation_rejects_drift() {
    let catalog = catalog_case("x.test.sh:0", &["bash-comparison"], false);
    let catalog_map = unique_catalog(std::slice::from_ref(&catalog)).unwrap();
    let mut valid = report(
        "bash-comparison",
        vec![reference_case("x.test.sh:0", ReferenceOutcome::Pass)],
    );
    assert!(validate_reference_report(&valid, &catalog_map, "commit").is_ok());

    valid.source_commit = "other".to_owned();
    assert!(validate_reference_report(&valid, &catalog_map, "commit").is_err());
    valid.source_commit = "commit".to_owned();
    valid.cases[0].line += 1;
    assert!(validate_reference_report(&valid, &catalog_map, "commit").is_err());
    valid.cases[0].line = catalog.line;
    valid.totals.pass = 0;
    assert!(validate_reference_report(&valid, &catalog_map, "commit").is_err());
}

#[test]
fn duplicate_case_ids_are_rejected() {
    let case = catalog_case("x.test.sh:0", &["bash-comparison"], false);
    assert!(unique_catalog(std::slice::from_ref(&case)).is_ok());
    assert!(
        unique_catalog(&[
            case,
            catalog_case("x.test.sh:0", &["bash-comparison"], false)
        ])
        .is_err()
    );

    let case = reference_case("x.test.sh:0", ReferenceOutcome::Pass);
    assert!(unique_report_cases(&report("bash-comparison", vec![case])).is_ok());
    let duplicates = report(
        "bash-comparison",
        vec![
            reference_case("x.test.sh:0", ReferenceOutcome::Pass),
            reference_case("x.test.sh:0", ReferenceOutcome::Pass),
        ],
    );
    assert!(unique_report_cases(&duplicates).is_err());
}

#[test]
fn observation_comparison_covers_fields() {
    let baseline = reference_case("x.test.sh:0", ReferenceOutcome::Pass);
    let mut changed = reference_case("x.test.sh:0", ReferenceOutcome::Pass);
    assert!(same_observation(&baseline, &changed));
    changed.status = Some(1);
    assert!(!same_observation(&baseline, &changed));
    changed.status = baseline.status;
    changed.qualifier = Some("BUG".to_owned());
    assert!(!same_observation(&baseline, &changed));
    changed.qualifier = baseline.qualifier.clone();
    changed.note = Some("different".to_owned());
    assert!(!same_observation(&baseline, &changed));
}

#[test]
fn group_profiles_preserve_overlaps() {
    let all = ["bash-comparison", "bash-extension", "bash-named-diagnostic"];
    let pass = catalog_case("a.test.sh:0", &all, false);
    let fail = catalog_case("b.test.sh:0", &all[..2], false);
    let unsupported = catalog_case("c.test.sh:0", &[all[0]], false);
    let cases = CaseManifest {
        schema: 1,
        oracle_version: ORACLE_VERSION.to_owned(),
        oracle_binary_sha256: "a".repeat(64),
        oils_commit: "commit".to_owned(),
        oils_tree: "tree".to_owned(),
        eligible: vec![pass.id.clone()],
        excluded: vec![
            excluded_case(&fail, Disposition::ReferenceFailure, RecordedOutcome::Fail),
            excluded_case(
                &unsupported,
                Disposition::Unsupported,
                RecordedOutcome::Unsupported,
            ),
        ],
    };
    let profiles = group_profiles_from_cases(&[pass, fail, unsupported], &cases).unwrap();
    assert_eq!(profiles[0].selected, 3);
    assert_eq!(profiles[0].eligible, 1);
    assert_eq!(profiles[0].reference_failure, 1);
    assert_eq!(profiles[0].unsupported, 1);
    assert_eq!(profiles[1].selected, 2);
    assert_eq!(profiles[1].eligible, 1);
    assert_eq!(profiles[1].reference_failure, 1);
    assert_eq!(profiles[2].selected, 1);
    assert_eq!(profiles[2].eligible, 1);
}

#[test]
fn build_and_runtime_profiles_are_explicit() {
    let build = expected_build_profile();
    assert_eq!(build.configure_args, configure_args());
    assert_eq!(build.cflags_template, CFLAGS_TEMPLATE);
    assert_eq!(build.environment["LC_ALL"], "C");
    assert_eq!(build.environment["TZ"], "UTC");
    assert_eq!(build.tools, pinned_tools());

    let execution = expected_execution_profile("sandbox-pid-net-ro-root".to_owned());
    assert!(execution.environment_is_cleared);
    assert_eq!(execution.environment["PATH"], "{fixture-bin}:/usr/bin:/bin");
    assert_eq!(execution.environment["LOCALE_ARCHIVE"], "");
    assert!(execution.unset.contains(&"HOME".to_owned()));
    assert!(execution.unset.contains(&"BASH_ENV".to_owned()));
}

#[test]
fn committed_profile_has_static_identity() {
    let root = crate::survey_root();
    let text = fs::read_to_string(root.join(PROFILE_FILE)).unwrap();
    let profile: ReferenceProfile = toml::from_str(&text).unwrap();
    assert_eq!(text, toml::to_string_pretty(&profile).unwrap());
    validate_static_profile(&profile).unwrap();
    assert_eq!(profile.patches.len(), 15);
    assert_eq!(profile.calibration.timeout_ms, 5_000);
}
