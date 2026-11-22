// Shared test-only Atlas checkpoint and held-out receipt fixture.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::{artifact, DomainId, DOMAIN_IDS};

const CANDIDATE_DIRECTORY: &str = "atlas-bert-candidates";
const FROZEN_FILE: &str = "atlas-bert-frozen.json";
const EVALUATION_FILE: &str = "atlas-bert-evaluation.json";
const FROZEN_SCHEMA: &str = "atlas-bert-frozen-v1";
const DATASET_SHA256: &str = "625f10150ba413f2fcae49ef425f9829004daaf9e69e49eecf66f7598de80d93";
const REGRESSION_POLICY: &str = "atlas-bert-synthetic-noninferiority-v1";
const EVALUATION_SCHEMA: &str = "atlas-bert-evaluation-v1";
const BASE_CHECKPOINT_REVISION: &str = "cfa538a0fddbbd978fefe8966c1aeff7ad409c90";
const PROTOCOL: &str = "argos-atlas-bert-v1";

pub(super) struct StagedCandidate {
    pub(super) home: tempfile::TempDir,
    pub(super) model_dir: PathBuf,
    pub(super) frozen_sha256: String,
    pub(super) evaluation_sha256: String,
}

fn write_checkpoint(directory: &Path, weights: &[u8]) -> BTreeMap<String, String> {
    fs::create_dir_all(directory).expect("create test checkpoint directory");
    let files: [(&str, &[u8]); 3] = [
        (
            "config.json",
            br#"{"routes":{"policy":[0.95,0.78,0.1,0.1,0.1,0.1]}}"#,
        ),
        ("vocab.txt", b"fixture vocabulary"),
        ("rust_model.ot", weights),
    ];
    files
        .into_iter()
        .map(|(name, bytes)| {
            fs::write(directory.join(name), bytes).expect("stage fixture checkpoint bytes");
            (name.to_owned(), artifact::sha256_hex(bytes))
        })
        .collect()
}

fn metrics(quality: f64, false_out_of_scope_admissions: usize) -> serde_json::Value {
    let tp = (10.0 * quality).round() as usize;
    let domains: BTreeMap<_, _> = DOMAIN_IDS
        .into_iter()
        .map(|name| {
            (
                name,
                json!({"tp": tp, "fp": 10-tp, "fn": 10-tp, "f1": quality}),
            )
        })
        .collect();
    json!({
        "macro_f1":quality,
        "micro_f1":quality,
        "false_out_of_scope_admissions":false_out_of_scope_admissions,
        "out_of_scope_rows":15,
        "per_domain":domains,
        "top_two":{
            "pair_rows":30,
            "exact_pair_rows":0,
            "triad_rows":20,
            "valid_triad_pairs":0,
            "single_domain_rows":120,
            "excess_single_domain_admissions":0,
            "published_pairs":0,
            "invalid_published_pairs":0,
        }
    })
}

impl StagedCandidate {
    pub(super) fn new(candidate_macro_f1: f64) -> Self {
        let home = tempfile::tempdir().expect("create an isolated test home");
        let root = artifact::root_for(home.path()).expect("load test artifact root");
        fs::create_dir_all(&root).expect("create the test artifact root");
        let baseline_dir = home.path().join("baseline");
        let baseline_assets = write_checkpoint(&baseline_dir, b"fixture baseline weights");
        let worker =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fixtures/atlas_bert");
        fs::write(
            root.join("zero-shot.json"),
            json!({"worker_path": worker, "model_dir": baseline_dir}).to_string(),
        )
        .expect("configure the baseline");
        let draft = home.path().join("draft");
        let candidate_assets = write_checkpoint(&draft, b"fixture candidate weights");
        let worker_sha256 = artifact::sha256_hex(&fs::read(worker).expect("read worker fixture"));
        let frozen = json!({
            "schema_version": FROZEN_SCHEMA,
            "dataset_sha256": DATASET_SHA256,
            "base_checkpoint_revision": BASE_CHECKPOINT_REVISION,
            "split_counts": {"train":440,"development":165,"calibration":230,"test":185},
            "threshold": 0.75,
            "assets": candidate_assets,
            "provenance": {
                "kind":"synthetic",
                "source_revision":"a".repeat(40),
                "calibration_worker_sha256":worker_sha256,
                "baseline_assets":baseline_assets,
                "runtime_contract":{
                    "protocol":PROTOCOL,
                    "domains":DOMAIN_IDS,
                    "hypotheses":DomainId::ALL.map(|domain| format!(
                        "This article substantially discusses the following topic: {}",
                        domain.definition()
                    )),
                    "max_tokens":192,
                    "max_batch":4,
                },
            },
        })
        .to_string();
        let frozen_sha256 = artifact::sha256_hex(frozen.as_bytes());
        let model_dir = root.join(CANDIDATE_DIRECTORY).join(&frozen_sha256);
        fs::create_dir_all(model_dir.parent().expect("candidate parent exists"))
            .expect("create fixture candidate root");
        fs::rename(draft, &model_dir).expect("stage fixture candidate");
        fs::write(model_dir.join(FROZEN_FILE), frozen).expect("write frozen fixture manifest");
        let evaluation = json!({
            "schema_version": EVALUATION_SCHEMA,
            "frozen_sha256": frozen_sha256,
            "dataset_sha256": DATASET_SHA256,
            "held_out_rows":185,
            "worker_sha256":worker_sha256,
            "baseline_threshold":0.8,
            "baseline_base_checkpoint_revision": BASE_CHECKPOINT_REVISION,
            "baseline_assets": baseline_assets,
            "regression_policy": REGRESSION_POLICY,
            "baseline": metrics(0.7, 2),
            "candidate": metrics(candidate_macro_f1, 1),
            "worker_wall_ms":{"baseline":1.0,"candidate":1.0},
            "passed":true,
        })
        .to_string();
        let evaluation_sha256 = artifact::sha256_hex(evaluation.as_bytes());
        fs::write(model_dir.join(EVALUATION_FILE), evaluation).expect("write fixture evaluation");
        Self {
            home,
            model_dir,
            frozen_sha256,
            evaluation_sha256,
        }
    }
}
