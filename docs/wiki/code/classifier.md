# Classifier runtime

[Code route](index.md) · [Taxonomy](../concepts/strategic-domains-taxonomy.md) · [Wiki](../../index.md)

@tag:classifier

Atlas uses a private Rust-BERT worker. The worker scores the six ordered domain hypotheses defined by the [taxonomy](../../../crates/argos-osint-core/src/atlas_taxonomy.rs). Each domain has an independent score. The admission policy allows at most two qualifying domains and abstains on ambiguous ties. A cold deployment uses the configured, pinned DistilBERT MNLI baseline. Only the backend operator can install a verified fine-tuned candidate.

## Runtime and model identity

The [standalone worker](../../../tools/atlas-bert) supplies LibTorch independently of the Fleetworks service host. The [classifier service](../../../services/api-host/src/classifier_service.rs) exposes authenticated private pin, validate, and inference routes for Distributed. A backend using the local workspace loads the same bounded model bundle and worker protocol. Workstation Status distinguishes reachability from verified inference. Registered native clients can read the persisted candidate model ID and activation revision through either transport. The declaration can be present before the worker verifies the checkpoint. The classic Atlas collection cycle is supported by the [Distributed overlay](../../../deploy/atlas-dispatch.md).

A run pins the model assets, worker, six-domain contract, and threshold. The fine-tuned model also pins the frozen manifest and held-out report. Existing BERT runs resume against their saved pin, including after a later activation. Keep their immutable assets and the configured baseline available. Candidate corruption or a missing exact pin fails closed.

## Fit and verify a candidate

Use the [pinned synthetic corpus](../../../datasets/argos_atlas_strategic_domains_v2.csv). Fit on 440 training articles and select weights on 165 development articles. Select the shared admission threshold on 230 calibration articles scored with the same Rust worker. Freeze the model bytes, exact dataset digest, threshold, baseline identity and provenance before scoring the 185 held-out articles.

The held-out report compares the candidate with the current pinned baseline using the actual Rust worker for both. Both the calibration partition and the independent held-out partition must score strictly above 65% macro F1. The report also includes per-domain and micro F1, false admissions for out-of-scope articles, and top-two behavior. A passing report qualifies the synthetic fixture for explicit promotion. Agreement with reviewed provider mappings on news cycles determines when to stop recalibrating.

On an isolated AMD64 Linux host with the pinned CPU Torch wheel from `verify_linux.py wheel` and the documented toolchain, run the complete proof from a clean revision.

```sh
python3 tools/atlas-bert/scripts/verify_linux.py run --finetune --agent --require-clean \
  --target x86_64-unknown-linux-gnu --revision "$revision" --output "$proof_dir"
```

For GitHub Actions, dispatch the existing Fleetworks workflow on the verified PR branch with `stage=standard`, `mode=pr`, `components=backend`, and the exact `head` and `base` SHAs. That explicit AMD64 proof adds the evaluated candidate under `proof/candidate/` while preserving the ordinary checkpoint under `model/`. Routine first and second passes verify the baseline and do not fit new weights.

## Explicit activation

Stage the candidate in the actual inference owner's classifier directory as `atlas-bert-candidates/<frozen_sha256>/`. The model, manifest and evaluation must be exactly the bytes from the qualified proof. For Distributed, run the operator command with write access to the classifier service's mounted workspace and the same worker paths and native libraries. The running private inference service reads the workspace without write access.

```sh
argos-api-host --role atlas-bert-revision
argos-api-host --role atlas-bert-activate \
  --candidate-sha256 "$frozen_sha256" --evaluation-sha256 "$evaluation_sha256" \
  --expected-bert-revision "$prior_revision"
argos-api-host --role atlas-bert-revision
```

The operator command keeps the writer lease from the expected revision check through publication. It verifies the candidate and its exact baseline on the configured inference worker before exchanging the active pointer. A cancellation observed before publication retains the prior model. The command waits for any in-flight commit to settle and returns the saved model identity. Re-read the active revision after an interrupted command before retrying. Fresh pins read the active pointer after restart; saved BERT runs retain their exact identity.

## Recalibrate on reviewed news cycles

Review the compiled NewsData and Currents category rules and the domain phrases that corroborate them. Use the SHA-256 digest from the rules you reviewed when you run recalibration.

```sh
argos-api-host --role atlas-bert-provider-mapping
```

Run recalibration from the Distributed operator workspace with an explicitly activated BERT candidate, deployed datastore credentials, and the same worker and native library paths as the classifier service.

```sh
argos-api-host --role atlas-bert-recalibrate \
  --expected-bert-revision "$prior_revision" \
  --reviewed-mapping-sha256 "$reviewed_mapping_sha256" \
  --review-cycle "$cycle"
```

Repeat `--review-cycle` to select up to eight completed cycles. Fleetworks reads their captured news and uses one eligible observation per story family. The operator reviews the mapping rules once for each digest. The command derives strategic-domain reference sets from those rules and leaves entirely unmapped observations out of scoring. Prior article reviews remain available separately.

Calibration selects the threshold on one set of families, and Test measures agreement on a separate set. Each split needs at least 20 families with a mapping match and 20 without a mapping match for each domain, among the families that have at least one mapped domain. Previously scored Test families stay reserved across attempts.

Both mapping-agreement macro F1 scores must exceed 65% to apply the threshold. An independent Test score strictly above 85% freezes recalibration for that checkpoint across restarts and reactivation. The result includes the reviewed mapping digest, cohort digest, and per-domain agreement counts. These counts measure agreement with provider-derived reference sets. The mapping leaves actual article membership unknown for unmatched domains, so this assessment provides no out-of-scope accuracy estimate.

## Historical Natural records

Existing Natural training and activation records keep their original engine, hashes and ownership. The gateway retains scoped historical reads. Shipped companions and service hosts exclude the Natural runner. Its vendored code and bundle generator remain available to the archived contract tests through a Cargo development feature. Both transports reject new Natural training and activation requests. Native setup validates and seeds the actor catalog and follows reader-visible index readiness. It does not stage an Atlas training file. [Workstation flow](native-connections.md)

## Linux deployment and retained standalone worker setup

Prepare the CPU worker and pinned English NLI checkpoint for the container's Linux architecture. The Linux helper above packages the worker, matching native libraries and verified assets with ABI/hash checks; the external operator supplies container-visible configuration. The worker runs offline once its assets are installed. The following standalone build instructions retain the earlier operator/direct-worker compatibility path, including its Intel macOS example; they are not setup steps for the Rune desktop and do not establish Linux deployment or installed-app acceptance.

1. Set up CPU LibTorch 2.1.0 and Python 3.11. The pinned PyTorch wheel supplies both the worker's LibTorch and the one-time model converter. On Intel macOS, install the Xcode Command Line Tools (`xcode-select --install`) and use an x86_64 Python and Rust toolchain. PyTorch 2.1.0 publishes an Intel macOS wheel. [PyTorch release files](https://pypi.org/project/torch/2.1.0/#files) · [tch 0.14.0 setup](https://docs.rs/crate/tch/0.14.0/source/README.md)

   On macOS, run the following in the shell you will use to build and run the native worker.

   ```sh
   python3.11 -m venv /absolute/path/atlas-bert-venv
   . /absolute/path/atlas-bert-venv/bin/activate
   python -m pip install 'numpy==1.26.4' 'torch==2.1.0'
   export LIBTORCH_USE_PYTORCH=1
   atlas_torch_lib=$(python -c 'import pathlib, torch; print(pathlib.Path(torch.__file__).parent / "lib")')
   export DYLD_LIBRARY_PATH="$atlas_torch_lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
   ```

   On Intel macOS, `rustc -vV` should report `host: x86_64-apple-darwin`. On Linux use the [PyTorch CPU wheel index](https://download.pytorch.org/whl/cpu) to install `torch==2.1.0` and set `LD_LIBRARY_PATH` in place of `DYLD_LIBRARY_PATH`. For a standalone CPU LibTorch 2.1.0 distribution, set `LIBTORCH` to its absolute root, omit `LIBTORCH_USE_PYTORCH`, and add its `lib` directory to the platform loader path. Keep the same loader environment when running the native worker.

2. From the repository root, prepare the [pinned Typeform DistilBERT MNLI checkpoint](https://huggingface.co/typeform/distilbert-base-uncased-mnli/tree/cfa538a0fddbbd978fefe8966c1aeff7ad409c90). Choose a new model directory.

   ```sh
   atlas_model_dir=/absolute/path/atlas-distilbert-mnli
   bash tools/atlas-bert/scripts/prepare-model.sh "$atlas_model_dir"
   ```

   The script verifies the source files and converts the weights with the Rust-BERT 0.22.0 converter and the worker's Cargo lockfile. Keep the resulting `config.json`, `vocab.txt`, and `rust_model.ot` together in the model directory.

3. From the repository root, build the separate worker.

   ```sh
   cargo build --release --locked --manifest-path tools/atlas-bert/Cargo.toml \
       --target-dir "$PWD/target/atlas-bert"
   ```

4. For Compose, stage the Linux executable, required LibTorch libraries and model under the Fleetworks workspace. Set the container-visible paths in workspace `classifier/zero-shot.json` as shown in the [Atlas overlay guide](../../../deploy/atlas-dispatch.md). The optional threshold defaults to `0.8` and accepts values in `[0.5, 1)`. Both slots use the same threshold.

5. Smoke-test the native worker directly before preparing the packaged assets.

   ```sh
   printf '%s\n' '{"texts":["A ministry signed a diplomatic agreement."]}' |
     /absolute/path/to/argos-atlas-bert serve /absolute/path/atlas-distilbert-mnli
   ```

   Start the explicit Atlas Compose overlay, submit a cycle through native Atlas and inspect its authorized job in Jobs. The classifier service can report transport readiness before its model assets are available. Atlas verifies the pin before a new collection and records configuration or inference errors on the job.

A run pins the executable, model bytes and admission threshold. Resume of acquisition or admission requires those exact assets. Proven derivative-only work can resume after the classifier is no longer loaded.

## Historical admission and provider monitoring

Historical requests are idempotent by request ID, scope and exact model pin. The accepted cutoff and upper occurrence key freeze the retained cohort. Each activation processes at most 64 occurrences, capturing original metadata and provenance before scoring. Publication uses the same conditional outcome transaction as foreground admission, with actual maintenance task authority. Source conflicts retain immutable captures; repeated material changes block that occurrence's job until explicit resume. Successful classification is followed by authoritative metadata, Related, actor and lifecycle receipt audits. A job cannot finish while affected derivatives are pending. Related readers validate native pair fingerprints, separately from retained legacy display hashes, against current admission/source/metadata before pagination; see the [Related revision contract](../concepts/atlas-classification-and-related-articles.md#phase-6-and-cross-cycle-related). Rejection hides new admitted indexes while preserving historical evidence and original cycle statistics. A saved pin is checked without starting JavaScript for already-current or derivative-only work.

NewsData v1 and Currents v1 categories are captured from existing responses before country, time, duplicate and admission filters. Metadata observations are deduplicated independently of exact per-fetch/cycle origins. Capture uses 64-row pages and bounded JSON; credentials, full article bodies and unknown extra fields are excluded. Excluded category labels are filtered individually, not treated as article rejection. Unknown vocabularies abstain; AI tags never become category labels.

Terminal cycle publication and one calibration job/outbox keyed by cycle and observation watermark commit together. Pause does not enqueue. Empty cohorts complete as `no_new_data`. New Atlas cohorts record provider monitoring separately from operator candidate fitting and promotion. The separate scheduler pool processes at most 512 observations per activation and yields between 64-row pages when foreground work appears. Observation dispositions and counters checkpoint atomically; replay and restart do not count the same origin twice. Story families retain permanent partitions, indexed bounded candidate discovery, and conservative quarantine for ambiguous merges or exhausted retention capacity. The page checks retained-family capacity once under its shared writer guard and accounts for each new family in that transaction.

The provider mapping currently has **no independently reviewed precision certificate**. Its partial labels remain monitoring/review material; no provider-only candidate fitting, threshold adjustment or automatic promotion is authorized. The existing masked learner and promotion validator require independently reviewed complete calibration/development evidence, sufficient families, pair quality and latency/memory comparisons before they can authorize replacement. Synthetic installs retain their active model. Runtime observation storage remains separate from the seed CSV.

## Independent derivative scheduling

Admission and the durable per-occurrence derivative job commit atomically. Collection completes after admission/handoff; Metadata, Related, Actors and Reconcile execute under the job’s own task lease. Collection pause/completion supplies no derivative authority and cannot cancel a separate job. Use `classifier derivative-job --job-id … --action status|pause|resume|cancel` for explicit controls. The scheduler resumes durable work on service startup; a short-lived CLI leaves pending work for the next startup.

One worker per home uses the existing serialized publisher. An activation currently dispatches one occurrence and at most one continuation, below the sixteen-unit ceiling, with a two-second dispatch budget checked before claim/dispatch. In-flight transactions may overrun; queue wait, publisher execution and traversal timings remain separate. Foreground requests receive priority with a fair derivative turn. Required source/body/catalog fences and atomic actor output, receipt, outbox and reconciliation handoff remain intact.

Committed publication also owns any remaining lifecycle cleanup. If the occurrence is restaged or rejected, a selectively queued cleanup job or a subsequent live derivative task resumes the saved post-publication frontier under its own lease. Cleanup preserves the original mutation and affected identities, verifies the still-current publication batch, and revalidates current source/support guards. New actor publication waits for earlier cleanup; stale receipts cannot authorize native output. Fresh rejection without an owed frontier queues no derivative work.

Run snapshots expose separate current-receipt actor and reconciliation readiness counters. Their denominator is admitted eligible occurrences, not all fetched articles. Collection completion, actor readiness, reconciliation readiness and index-outbox delivery are distinct facts. Native actor index receipts bind the publication generation as well as the current input revision. Topology, actor discovery and provenance readers exclude stale native mentions and relationships while deferred cleanup is pending. Curated, pinned and validation-bearing evidence retains its own authority. Topology discovers both current graph projections and retained legacy endpoint indexes. Background scheduling is not evidence of faster total convergence; fixed-cohort timings are still required.

## Review tooling

Bounded exports contain captured observations, source hashes, permanent family/split ownership and explicit sampling bias; ground truth starts unreviewed. Imports require actual reviewer references, complete zero-to-six memberships or abstention, and matching source/revision/partition. Atomic replay is idempotent; conflicts roll back the exchange. Mapping approval derives precision counts from complete reviewed Development families. Missing provider categories remain Unknown. Approval alone neither activates a model nor proves production quality.
