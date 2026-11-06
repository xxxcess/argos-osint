//! Independent synthetic fixtures for the public CSV selection/validation contract.
use std::fs;
use std::path::Path;

use argos_osint_core::classifier::dataset::{
    self, CsvProfile, DatasetCandidate, DatasetError, DatasetRequest, ParsedDataset,
};
use argos_osint_core::classifier::training::{self, LabelMembership};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const FIELDS: [&str; 18] = [
    "sample_id",
    "title",
    "description",
    "domain_labels",
    "primary_domain",
    "secondary_domain",
    "subcategories",
    "source_kind",
    "difficulty",
    "dataset_version",
    "annotation_notes",
    "split",
    "scenario_group",
    "language",
    "synthetic",
    "labels_complete",
    "expected_admission",
    "rejection_reason",
];

fn row(id: &str, family: &str, split: &str, domains: &[&str]) -> Vec<String> {
    let (difficulty, admission, rejection) = match domains.len() {
        0 => ("hard_negative", "reject", "out_of_scope"),
        1 => ("single_domain_boundary", "reject", "insufficient_domains"),
        _ => ("cross_domain", "admit", ""),
    };
    vec![
        id.into(),
        format!("Synthetic bulletin {id}"),
        "Fictional report, with \"quoted\" detail.\nSecond sentence.".into(),
        domains.join("|"),
        domains.first().copied().unwrap_or("").into(),
        domains.get(1).copied().unwrap_or("").into(),
        "".into(),
        "synthetic".into(),
        difficulty.into(),
        "synthetic-development-v1".into(),
        "Ignored annotation".into(),
        split.into(),
        family.into(),
        "en".into(),
        "true".into(),
        "true".into(),
        admission.into(),
        rejection.into(),
    ]
}
fn encode(rows: &[Vec<String>]) -> Vec<u8> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer.write_record(FIELDS).expect("fixture header");
    for row in rows {
        writer.write_record(row).expect("fixture row");
    }
    writer.into_inner().expect("fixture bytes")
}
fn request(root: &Path) -> DatasetRequest {
    let mut request = DatasetRequest::default();
    request.cwd = root.to_path_buf();
    request.executable = root.join("argos");
    request.home = root.join("home");
    request
}
fn parse_bytes(bytes: &[u8]) -> Result<ParsedDataset, DatasetError> {
    let root = TempDir::new().expect("isolated dataset directory");
    fs::write(root.path().join("fixture.csv"), bytes).expect("write fixture");
    let mut req = request(root.path());
    req.explicit_file = Some("fixture.csv".into());
    let selected = dataset::resolve_dataset(&req)?;
    dataset::parse_csv(&selected)
}
fn development() -> Vec<Vec<String>> {
    vec![
        row("sample-a", "family-a", "train", &["governance", "security"]),
        row("sample-b", "family-b", "calibration", &["economic"]),
        row("sample-c", "family-c", "test", &[]),
    ]
}

#[test]
fn configured_csv_is_authoritative_after_explicit_and_environment_paths() {
    let root = TempDir::new().unwrap();
    let mut request = request(root.path());
    fs::write(root.path().join("saved.csv"), encode(&development())).unwrap();
    request.configured_file = Some(root.path().join("saved.csv"));
    assert_eq!(
        dataset::resolve_dataset(&request).unwrap().candidate,
        DatasetCandidate::Configured
    );
    request.environment_file = Some(root.path().join("saved.csv"));
    assert_eq!(
        dataset::resolve_dataset(&request).unwrap().candidate,
        DatasetCandidate::Environment
    );
    request.explicit_file = Some(root.path().join("saved.csv"));
    assert_eq!(
        dataset::resolve_dataset(&request).unwrap().candidate,
        DatasetCandidate::Explicit
    );
    request.explicit_file = None;
    request.environment_file = None;
    fs::remove_file(root.path().join("saved.csv")).unwrap();
    assert!(
        dataset::resolve_dataset(&request).is_err(),
        "a missing configured file cannot silently select a default"
    );
}

#[test]
fn rfc_quotes_and_newlines_preserve_features_and_exact_hash() {
    let rows = development();
    let bytes = encode(&rows);
    let parsed = parse_bytes(&bytes).expect("valid independently generated CSV");
    assert_eq!(parsed.profile, CsvProfile::SyntheticDevelopment);
    assert_eq!(parsed.corpus.rows[0].description, rows[0][2]);
    assert_eq!(
        parsed.corpus.rows[0].membership,
        [
            LabelMembership::Present,
            LabelMembership::Present,
            LabelMembership::Absent,
            LabelMembership::Absent,
            LabelMembership::Absent,
            LabelMembership::Absent
        ]
    );
    assert_eq!(parsed.summary.split_rows, [1, 1, 0, 1]);
    assert_eq!(
        parsed.summary.dataset_sha256,
        format!("{:x}", Sha256::digest(&bytes))
    );
    let expected = training::corpus_hashes(&parsed.corpus.rows).expect("canonical corpus hash");
    assert_eq!(
        parsed.corpus.family_assignment_sha256,
        expected.family_assignment_sha256
    );
    assert_eq!(
        parsed.corpus.train_split_sha256,
        expected.train_split_sha256
    );
    assert!(!parsed.corpus.rows[0]
        .description
        .contains("Ignored annotation"));
}

#[test]
fn reordered_columns_bom_and_crlf_are_supported_without_hash_normalization() {
    let rows = development();
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    writer
        .write_record(FIELDS.iter().rev())
        .expect("reordered header");
    for row in &rows {
        writer
            .write_record(row.iter().rev())
            .expect("reordered row");
    }
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend(writer.into_inner().expect("CSV bytes"));
    let parsed = parse_bytes(&bytes).expect("BOM/reordered CRLF CSV");
    assert_eq!(parsed.corpus.rows[0].title, rows[0][1]);
    assert_eq!(
        parsed.summary.dataset_sha256,
        format!("{:x}", Sha256::digest(&bytes))
    );
}

#[test]
fn header_requires_the_exact_unique_field_set() {
    let encoded = encode(&development());
    let text = String::from_utf8(encoded).expect("UTF8 fixture");
    for replacement in ["unknown", "title", ""] {
        let malformed = text.replacen("sample_id", replacement, 1);
        assert!(
            matches!(
                parse_bytes(malformed.as_bytes()),
                Err(DatasetError::Schema { .. })
            ),
            "header replacement {replacement:?} must fail"
        );
    }
}

#[test]
fn invalid_annotations_never_reach_training_corpus() {
    for (field, invalid, code) in [
        (3, "governance|governance", "duplicate_domain"),
        (3, "unk", "unknown_domain"),
        (3, "governance|", "unknown_domain"),
        (3, " governance", "unknown_domain"),
        (14, "false", "invalid_boolean"),
        (15, "TRUE", "invalid_boolean"),
        (7, "reviewed_real", "unsupported_provenance"),
        (11, "development", "unsupported_split"),
        (13, "fr", "unsupported_language"),
        (16, "reject", "annotation_mismatch"),
        (17, "out_of_scope", "annotation_mismatch"),
        (8, "single_domain_boundary", "annotation_mismatch"),
        (4, "human", "annotation_mismatch"),
        (9, "", "invalid_identifier"),
    ] {
        let mut rows = development();
        rows[0][field] = invalid.into();
        match parse_bytes(&encode(&rows)) {
            Err(DatasetError::InvalidRow { code: actual, .. }) => {
                assert_eq!(actual, code, "field {}", FIELDS[field])
            }
            other => panic!("field {} expected {code}, got {other:?}", FIELDS[field]),
        }
    }
}

#[test]
fn family_partition_and_membership_conflicts_fail_instead_of_resplitting() {
    for (split, domains, code) in [
        (
            "test",
            vec!["governance", "security"],
            "family_split_leakage",
        ),
        ("train", vec!["human"], "family_label_conflict"),
    ] {
        let mut rows = development();
        rows.push(row("sample-d", "family-a", split, &domains));
        assert!(
            matches!(parse_bytes(&encode(&rows)), Err(DatasetError::FamilyConflict { code: actual, .. }) if actual == code)
        );
    }
    let mut rows = development();
    rows[1][0] = rows[0][0].clone();
    assert!(
        matches!(parse_bytes(&encode(&rows)), Err(DatasetError::InvalidRow { code, .. }) if code == "duplicate_id")
    );
}

#[test]
fn malformed_quotes_utf8_and_field_counts_fail_closed() {
    let header = format!("{}\n", FIELDS.join(","));
    for tail in [
        b"\"unterminated".as_slice(),
        b"a\"bad,b".as_slice(),
        b"\"closed\"junk,b".as_slice(),
        b"\xff,b".as_slice(),
    ] {
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend(tail);
        assert!(matches!(parse_bytes(&bytes), Err(DatasetError::Csv { .. })));
    }
    let mut rows = development();
    rows[0].pop();
    let mut writer = csv::WriterBuilder::new()
        .flexible(true)
        .from_writer(Vec::new());
    writer.write_record(FIELDS).expect("header");
    writer.write_record(&rows[0]).expect("short row");
    assert!(
        matches!(parse_bytes(&writer.into_inner().expect("bytes")), Err(DatasetError::Csv { code, .. }) if code == "field_count")
    );
}

#[test]
fn raw_record_and_decoded_metadata_bounds_are_distinct() {
    let mut bytes = format!("{}\n\"", FIELDS.join(",")).into_bytes();
    bytes.extend(vec![b'x'; dataset::MAX_CSV_RECORD_BYTES]);
    assert!(
        matches!(parse_bytes(&bytes), Err(DatasetError::TooLarge { kind, .. }) if kind == "record_bytes")
    );
    let mut rows = development();
    rows[0][10] = "x".repeat(dataset::MAX_METADATA_FIELD_BYTES + 1);
    assert!(
        matches!(parse_bytes(&encode(&rows)), Err(DatasetError::TooLarge { kind, .. }) if kind == "field_annotation_notes")
    );
    let mut rows = development();
    rows[0][1] = "x".repeat(training::MAX_ROW_TEXT_BYTES);
    assert!(
        matches!(parse_bytes(&encode(&rows)), Err(DatasetError::TooLarge { kind, .. }) if kind == "row_text_bytes")
    );
}

#[test]
fn explicit_selection_is_authoritative_even_when_automatic_data_exists() {
    let root = TempDir::new().expect("isolated resolver root");
    fs::create_dir_all(root.path().join("home/dataset")).expect("home data directory");
    fs::write(
        root.path()
            .join("home/dataset")
            .join(dataset::DATASET_FILE_NAME),
        encode(&development()),
    )
    .expect("automatic data");
    let mut req = request(root.path());
    req.explicit_file = Some("missing.csv".into());
    assert!(dataset::resolve_dataset(&req).is_err());
    req.explicit_file = Some("".into());
    assert!(matches!(
        dataset::resolve_dataset(&req),
        Err(DatasetError::InvalidSelection { .. })
    ));
    req.explicit_file = None;
    req.environment_file = Some("".into());
    assert!(matches!(
        dataset::resolve_dataset(&req),
        Err(DatasetError::InvalidSelection { .. })
    ));
    req.explicit_file = Some(
        root.path()
            .join("home/dataset")
            .join(dataset::DATASET_FILE_NAME),
    );
    assert_eq!(
        dataset::resolve_dataset(&req)
            .expect("explicit beats bad env")
            .candidate,
        DatasetCandidate::Explicit
    );
}

#[test]
fn runtime_workspace_order_and_installed_home_fallback_need_no_git_or_developer_path() {
    let root = TempDir::new().expect("isolated workspace");
    fs::create_dir_all(root.path().join("crates/argos-osint-core")).expect("marker directory");
    fs::write(
        root.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/argos-osint-core\"]\n",
    )
    .expect("workspace marker");
    fs::write(
        root.path().join("crates/argos-osint-core/Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("crate marker");
    for dir in ["dataset", "datasets", "home/dataset"] {
        fs::create_dir_all(root.path().join(dir)).expect("candidate directory");
        fs::write(
            root.path().join(dir).join(dataset::DATASET_FILE_NAME),
            encode(&development()),
        )
        .expect("candidate CSV");
    }
    let req = request(root.path());
    assert_eq!(
        dataset::resolve_dataset(&req)
            .expect("singular first")
            .candidate,
        DatasetCandidate::RepoSingular
    );
    fs::remove_file(root.path().join("dataset").join(dataset::DATASET_FILE_NAME))
        .expect("remove singular fixture");
    assert_eq!(
        dataset::resolve_dataset(&req)
            .expect("plural second")
            .candidate,
        DatasetCandidate::RepoPlural
    );
    fs::remove_file(root.path().join("Cargo.toml")).expect("remove marker");
    assert_eq!(
        dataset::resolve_dataset(&req)
            .expect("home outside workspace")
            .candidate,
        DatasetCandidate::Home
    );
}

#[test]
fn executable_workspace_discovery_is_secondary_to_cwd_and_rejects_nonregular_candidates() {
    let root = TempDir::new().expect("isolated runtime layout");
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join("crates/argos-osint-core")).expect("core marker parent");
    fs::create_dir_all(repo.join("bin")).expect("executable directory");
    fs::create_dir_all(repo.join("datasets")).expect("dataset directory");
    fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers=[\"crates/argos-osint-core\"]\n",
    )
    .expect("workspace marker");
    fs::write(repo.join("crates/argos-osint-core/Cargo.toml"), "").expect("core marker");
    fs::write(repo.join("bin/argos"), "").expect("runtime executable fixture");
    fs::write(
        repo.join("datasets").join(dataset::DATASET_FILE_NAME),
        encode(&development()),
    )
    .expect("repo CSV");
    let mut req = request(root.path());
    req.executable = repo.join("bin/argos");
    assert_eq!(
        dataset::resolve_dataset(&req)
            .expect("exe root fallback")
            .candidate,
        DatasetCandidate::RepoPlural
    );
    fs::create_dir_all(repo.join("dataset").join(dataset::DATASET_FILE_NAME))
        .expect("nonregular candidate");
    assert!(
        matches!(dataset::resolve_dataset(&req), Err(DatasetError::InvalidSelection { reason, .. }) if reason == "not_regular_file")
    );
}

#[test]
fn changed_selection_aborts_before_parsing_instead_of_using_stale_fingerprint() {
    let root = TempDir::new().expect("isolated file");
    let path = root.path().join("fixture.csv");
    fs::write(&path, encode(&development())).expect("fixture CSV");
    let mut req = request(root.path());
    req.explicit_file = Some(path.clone());
    let resolution = dataset::resolve_dataset(&req).expect("select original");
    fs::write(&path, b"changed").expect("change selected fixture");
    assert!(matches!(
        dataset::parse_csv(&resolution),
        Err(DatasetError::ChangedDuringRead { .. })
    ));
}

fn release() -> Vec<Vec<String>> {
    let domains = [
        "governance",
        "security",
        "economic",
        "human",
        "environmental",
        "scientific",
    ];
    let mut rows = Vec::new();
    let mut add_family = |family: String, split: &str, labels: &[&str], variants: usize| {
        for variant in 0..variants {
            let mut record = row(&format!("{family}-v{variant}"), &family, split, labels);
            record[9] = dataset::SEED_DATASET_VERSION.into();
            rows.push(record);
        }
    };
    for domain in domains {
        for family in 0..20 {
            add_family(
                format!("focused-{domain}-{family}"),
                if family < 14 {
                    "train"
                } else if family < 17 {
                    "calibration"
                } else {
                    "test"
                },
                &[domain],
                10,
            );
        }
    }
    for (i, first) in domains.iter().enumerate() {
        for second in domains.iter().skip(i + 1) {
            for family in 0..4 {
                add_family(
                    format!("overlap-{first}-{second}-{family}"),
                    if family < 2 {
                        "train"
                    } else if family == 2 {
                        "calibration"
                    } else {
                        "test"
                    },
                    &[*first, *second],
                    5,
                );
            }
        }
    }
    for family in 0..40 {
        add_family(
            format!("negative-{family}"),
            if family < 28 {
                "train"
            } else if family < 34 {
                "calibration"
            } else {
                "test"
            },
            &[],
            5,
        );
    }
    rows
}

#[test]
fn delivered_version_requires_complete_independent_release_invariants() {
    let mut rows = release();
    let parsed = parse_bytes(&encode(&rows)).expect("independent release recipe");
    assert_eq!(parsed.profile, CsvProfile::DeliveredSeedV1);
    assert_eq!(parsed.summary.rows, 1700);
    assert_eq!(parsed.summary.families, 220);
    assert_eq!(parsed.summary.split_rows, [1130, 285, 0, 285]);
    assert_eq!(parsed.summary.split_families, [142, 39, 0, 39]);
    assert_eq!(
        parsed.summary.positive_rows,
        [[190; 6], [55; 6], [0; 6], [55; 6]]
    );
    rows.pop();
    assert!(matches!(
        parse_bytes(&encode(&rows)),
        Err(DatasetError::ReleaseCounts { .. })
    ));
}

#[test]
fn release_family_cardinality_is_not_proven_by_aggregate_row_counts() {
    let mut rows = release();
    // Move one variant between same-domain train families: aggregate labels,
    // rows and family counts remain valid, but independent family shape fails.
    rows[0][12] = rows[10][12].clone();
    assert!(
        matches!(parse_bytes(&encode(&rows)), Err(DatasetError::ReleaseCounts { invariant, .. }) if invariant == "focused_family_variants")
    );
}

#[test]
fn delivered_distinct_corpus_keeps_four_partitions_and_complete_triads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut req = request(&root);
    req.explicit_file = Some(root.join("datasets/argos_atlas_strategic_domains_v2.csv"));
    let parsed = dataset::parse_csv(&dataset::resolve_dataset(&req).unwrap()).unwrap();
    assert_eq!(parsed.profile, CsvProfile::DeliveredDistinctV2);
    assert_eq!(parsed.summary.rows, 1020);
    assert_eq!(parsed.summary.split_families, [440, 230, 165, 185]);
    assert_eq!(
        parsed.summary.positive_rows,
        [[80; 6], [40; 6], [30; 6], [40; 6]]
    );
    assert_eq!(
        parsed.corpus.development_split_sha256,
        training::corpus_hashes(&parsed.corpus.rows)
            .unwrap()
            .development_split_sha256
    );
    assert_eq!(
        parsed
            .corpus
            .rows
            .iter()
            .filter(|row| row
                .membership
                .iter()
                .filter(|m| **m == LabelMembership::Present)
                .count()
                == 3)
            .count(),
        20
    );
}

#[test]
fn unknown_csv_policy_version_is_rejected_explicitly() {
    let mut rows = development();
    for row in &mut rows {
        row[9] = "unknown-future-policy".into();
    }
    assert!(
        matches!(parse_bytes(&encode(&rows)), Err(DatasetError::InvalidRow {code,..}) if code=="unsupported_dataset_version")
    );
}

#[test]
fn development_content_changes_its_hash_without_changing_training_identity() {
    let mut rows = development();
    for row in &mut rows {
        row[9] = "synthetic-development-v2".into();
    }
    rows[1][16] = "admit".into();
    rows[1][17] = "".into();
    rows.push(row("dev", "dev-family", "development", &["human"]));
    let dev = rows.last_mut().unwrap();
    dev[9] = "synthetic-development-v2".into();
    dev[16] = "admit".into();
    dev[17] = "".into();
    let first = parse_bytes(&encode(&rows)).unwrap();
    rows.last_mut().unwrap()[2] = "Changed held-out development content".into();
    let second = parse_bytes(&encode(&rows)).unwrap();
    assert_eq!(
        first.corpus.train_split_sha256,
        second.corpus.train_split_sha256
    );
    assert_ne!(
        first.corpus.development_split_sha256,
        second.corpus.development_split_sha256
    );
}
