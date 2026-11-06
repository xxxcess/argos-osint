//! Pinned local six-domain Atlas admission and raw-only computation cache. @tag:classifier @tag:atlas-collection
pub use local::{
    classify_page, AdmissionContext, CacheClock, CacheMetrics, CachedComputation, ComputationCache,
    ComputationKey, ItemClassification, MonotonicClock, OccurrenceKey, OccurrenceProvenance,
    PageClassification, PageMetrics, ReceiptKey, StoredReceipt, CACHE_BYTES, CACHE_ENTRIES,
    CACHE_SCHEMA_VERSION, CACHE_TTL, RECEIPT_KEY_VERSION,
};
mod local {
    // Local six-domain Atlas admission and raw-computation cache. @tag:classifier
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde::{Deserialize, Serialize};

    use crate::classifier::artifact::{sha256_hex, ArtifactStore};
    use crate::classifier::calibration::{self, GateSet};
    use crate::classifier::readiness::PinnedClassifier;
    use crate::classifier::training::TrainingReport;
    use crate::classifier::{
        ClassificationReceipt, ClassifierError, ClassifyInput, RawDomainScore, WorkControl,
    };
    use crate::storage::classifier_admission::{
        AdmissionClaim, AdmissionError, ClaimedInput, PUBLICATION_VERSION,
    };
    use crate::storage::codec;

    pub const CACHE_SCHEMA_VERSION: &str = "atlas-classifier-computation-v1";
    pub const RECEIPT_KEY_VERSION: &str = "atlas-occurrence-receipt-v1";
    pub const CACHE_ENTRIES: usize = 4096;
    pub const CACHE_BYTES: usize = 8 * 1024 * 1024;
    pub const CACHE_TTL: Duration = Duration::from_secs(3600);

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[non_exhaustive]
    pub struct OccurrenceKey {
        pub run_id: String,
        pub article_id: String,
    }
    impl OccurrenceKey {
        #[must_use]
        pub fn new(run_id: impl Into<String>, article_id: impl Into<String>) -> Self {
            Self {
                run_id: run_id.into(),
                article_id: article_id.into(),
            }
        }
        #[must_use]
        pub fn record_id(&self) -> String {
            codec::encode(&[&self.run_id, &self.article_id])
        }
    }
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[non_exhaustive]
    pub struct ReceiptKey {
        pub key_version: String,
        pub occurrence: OccurrenceKey,
        pub input_revision: u64,
        pub source_revision: String,
        pub classifier_input_hash: String,
        pub model_id: String,
        pub artifact_sha256: String,
        pub manifest_sha256: String,
        pub taxonomy_version: String,
        pub preprocessing_version: String,
        pub policy_version: String,
        pub calibration_version: String,
        pub receipt_schema_version: String,
    }
    impl ReceiptKey {
        #[must_use]
        pub fn record_id(&self) -> String {
            codec::encode(&[
                &self.key_version,
                &self.occurrence.run_id,
                &self.occurrence.article_id,
                &self.input_revision.to_string(),
                &self.source_revision,
                &self.classifier_input_hash,
                &self.model_id,
                &self.artifact_sha256,
                &self.manifest_sha256,
                &self.taxonomy_version,
                &self.preprocessing_version,
                &self.policy_version,
                &self.calibration_version,
                &self.receipt_schema_version,
            ])
        }
    }
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct OccurrenceProvenance {
        pub provider: String,
        pub url: String,
        pub source_domain: String,
        pub source_name: String,
        pub published_at: String,
        pub seen_at: String,
        pub language: Option<String>,
        pub observation_ids: Vec<String>,
    }
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct StoredReceipt {
        pub key: ReceiptKey,
        pub receipt: ClassificationReceipt,
        pub provenance: OccurrenceProvenance,
        pub publication_version: String,
    }
    impl StoredReceipt {
        /// Checks the complete reusable identity before storage or cache reuse.
        pub fn validate(&self) -> Result<(), AdmissionError> {
            self.receipt.validate()?;
            if self.key.key_version != RECEIPT_KEY_VERSION
                || self.publication_version != PUBLICATION_VERSION
                || self.key.taxonomy_version != crate::atlas_taxonomy::TAXONOMY_VERSION
                || self.key.preprocessing_version != crate::classifier::PREPROCESSING_VERSION
                || self.key.policy_version != crate::classifier::ADMISSION_POLICY_VERSION
                || self.key.calibration_version != calibration::CALIBRATION_VERSION
                || self.key.receipt_schema_version != crate::classifier::RECEIPT_SCHEMA_VERSION
            {
                return Err(AdmissionError::UnsupportedVersion {
                    kind: "receipt".into(),
                    found: self.key.key_version.clone(),
                });
            }
            if self.key.input_revision == 0
                || self.key.input_revision > i64::MAX as u64
                || self.key.occurrence.run_id.is_empty()
                || self.key.occurrence.article_id.is_empty()
                || self.receipt.occurrence_id != self.key.occurrence.record_id()
                || self.receipt.input_hash != self.key.classifier_input_hash
                || self.receipt.model_id != self.key.model_id
                || self.receipt.taxonomy_version != self.key.taxonomy_version
                || self.receipt.preprocessing_version != self.key.preprocessing_version
                || self.receipt.policy_version != self.key.policy_version
                || self.receipt.receipt_schema_version != self.key.receipt_schema_version
                || [
                    self.key.artifact_sha256.as_str(),
                    self.key.manifest_sha256.as_str(),
                    self.key.classifier_input_hash.as_str(),
                ]
                .iter()
                .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(AdmissionError::InvalidReceipt {
                    code: "identity_mismatch".into(),
                });
            }
            Ok(())
        }
    }
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct ComputationKey {
        pub cache_schema_version: String,
        pub input_hash: String,
        pub model_id: String,
        pub artifact_sha256: String,
        pub manifest_sha256: String,
        pub preprocessing_version: String,
        pub policy_version: String,
        pub calibration_version: String,
    }
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct CachedComputation {
        pub key: ComputationKey,
        pub raw: [RawDomainScore; 6],
    }
    /// Injected monotonic clock: cache expiry never relies on a wall clock.
    pub trait CacheClock: Send + Sync {
        fn now(&self) -> Duration;
    }
    pub struct MonotonicClock {
        origin: Instant,
    }
    impl Default for MonotonicClock {
        fn default() -> Self {
            Self {
                origin: Instant::now(),
            }
        }
    }
    impl CacheClock for MonotonicClock {
        fn now(&self) -> Duration {
            self.origin.elapsed()
        }
    }
    struct CacheEntry {
        computation: CachedComputation,
        inserted: Duration,
        touched: u64,
        bytes: usize,
    }
    #[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct CacheMetrics {
        pub hits: u64,
        pub misses: u64,
        pub evictions: u64,
        pub bytes: usize,
    }
    /// Correctness-neutral bounded cache; occurrence identity/provenance never enters a value.
    pub struct ComputationCache {
        clock: Arc<dyn CacheClock>,
        entries: BTreeMap<ComputationKey, CacheEntry>,
        tick: u64,
        metrics: CacheMetrics,
    }
    impl Default for ComputationCache {
        fn default() -> Self {
            Self::new(Arc::new(MonotonicClock::default()))
        }
    }
    impl ComputationCache {
        #[must_use]
        pub fn new(clock: Arc<dyn CacheClock>) -> Self {
            Self {
                clock,
                entries: BTreeMap::new(),
                tick: 0,
                metrics: CacheMetrics::default(),
            }
        }
        #[must_use]
        pub fn metrics(&self) -> CacheMetrics {
            self.metrics
        }
        fn next_tick(&mut self) -> u64 {
            if self.tick == u64::MAX {
                let mut keys: Vec<_> = self
                    .entries
                    .iter()
                    .map(|(key, value)| (value.touched, key.clone()))
                    .collect();
                keys.sort();
                for (index, (_, key)) in keys.into_iter().enumerate() {
                    if let Some(entry) = self.entries.get_mut(&key) {
                        entry.touched = index as u64;
                    }
                }
                self.tick = self.entries.len() as u64;
            }
            self.tick += 1;
            self.tick
        }
        fn evict(&mut self, key: &ComputationKey) {
            if let Some(entry) = self.entries.remove(key) {
                self.metrics.bytes -= entry.bytes;
                self.metrics.evictions = self.metrics.evictions.saturating_add(1);
            }
        }
        fn expire(&mut self) {
            let now = self.clock.now();
            let expired: Vec<_> = self
                .entries
                .iter()
                .filter(|(_, entry)| {
                    now.checked_sub(entry.inserted)
                        .is_none_or(|age| age >= CACHE_TTL)
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in expired {
                self.evict(&key);
            }
        }
        pub fn get(&mut self, key: &ComputationKey) -> Option<[RawDomainScore; 6]> {
            self.expire();
            let tick = self.next_tick();
            if let Some(entry) = self.entries.get_mut(key) {
                entry.touched = tick;
                self.metrics.hits = self.metrics.hits.saturating_add(1);
                Some(entry.computation.raw)
            } else {
                self.metrics.misses = self.metrics.misses.saturating_add(1);
                None
            }
        }
        pub fn insert(&mut self, computation: CachedComputation) -> Result<(), AdmissionError> {
            let bytes = crate::storage::classifier_admission::bounded_json(
                &computation,
                CACHE_BYTES,
                "cache_computation",
            )?
            .len();
            if computation.key.cache_schema_version != CACHE_SCHEMA_VERSION
                || computation
                    .raw
                    .iter()
                    .zip(crate::atlas_taxonomy::DomainId::ALL)
                    .any(|(raw, domain)| {
                        raw.domain != domain
                            || !raw.raw_log_odds.is_finite()
                            || !raw.present_log_score.is_finite()
                            || !raw.absent_log_score.is_finite()
                            || !(raw.present_log_score - raw.absent_log_score).is_finite()
                            || (raw.raw_log_odds - (raw.present_log_score - raw.absent_log_score))
                                .abs()
                                > 1e-12
                    })
            {
                return Err(AdmissionError::InvalidReceipt {
                    code: "invalid_cached_raw_scores".into(),
                });
            }
            self.expire();
            if self.entries.contains_key(&computation.key) {
                self.evict(&computation.key);
            }
            while self.entries.len() >= CACHE_ENTRIES || self.metrics.bytes + bytes > CACHE_BYTES {
                let key = self
                    .entries
                    .iter()
                    .min_by(|(ak, a), (bk, b)| a.touched.cmp(&b.touched).then_with(|| ak.cmp(bk)))
                    .map(|(key, _)| key.clone());
                if let Some(key) = key {
                    self.evict(&key);
                } else {
                    break;
                }
            }
            let touched = self.next_tick();
            self.metrics.bytes += bytes;
            self.entries.insert(
                computation.key.clone(),
                CacheEntry {
                    computation,
                    inserted: self.clock.now(),
                    touched,
                    bytes,
                },
            );
            Ok(())
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct PageMetrics {
        pub occurrences: u64,
        pub computations: u64,
        /// Observed title/description String capacities owned by the local
        /// adapter, including retained input and raw-request copies.
        pub owned_text_buffer_peak_bytes: u64,
        pub cache: CacheMetrics,
        pub rejections: BTreeMap<String, u64>,
        pub operational_errors: BTreeMap<String, u64>,
    }
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct PageClassification {
        pub outcomes: Vec<ItemClassification>,
        pub metrics: PageMetrics,
    }
    #[derive(Clone)]
    #[non_exhaustive]
    pub struct AdmissionContext {
        pub pin: PinnedClassifier,
        pub gates: GateSet,
        pub manifest_sha256: String,
    }
    impl AdmissionContext {
        /// Loads only this exact immutable model/report. Call off the Tokio worker.
        pub fn load(home: &Path, pin: PinnedClassifier) -> Result<Self, AdmissionError> {
            let store = ArtifactStore::new(home)?;
            let id = crate::classifier::artifact::ModelId::parse(&pin.snapshot.model_id)
                .ok_or_else(|| AdmissionError::InvalidReceipt {
                    code: "invalid_model_id".into(),
                })?;
            let artifact = store.validate_model_integrity(&id)?;
            let manifest = store.read_manifest_raw(&id)?;
            let manifest_sha256 = sha256_hex(&manifest);
            let report: TrainingReport =
                serde_json::from_str(&store.read_report(&id)?).map_err(|_| {
                    AdmissionError::InvalidReceipt {
                        code: "invalid_model_report".into(),
                    }
                })?;
            if manifest_sha256 != pin.manifest_sha256
                || artifact.manifest.model_id.as_str() != pin.snapshot.model_id
                || artifact.manifest.model_sha256 != pin.snapshot.artifact_sha256
                || artifact.manifest.taxonomy_version != pin.snapshot.taxonomy_version
                || artifact.manifest.preprocessing_version != pin.snapshot.preprocessing_version
                || artifact.manifest.admission_policy_version != pin.snapshot.policy_version
                || artifact.manifest.algorithm_version != pin.snapshot.algorithm_version
                || artifact.manifest.provenance != pin.snapshot.provenance
            {
                return Err(AdmissionError::InvalidReceipt {
                    code: "pinned_model_identity_mismatch".into(),
                });
            }
            Ok(Self {
                pin,
                gates: report.gate_set,
                manifest_sha256,
            })
        }
        fn computation_key(&self, input: &ClassifyInput) -> ComputationKey {
            ComputationKey {
                cache_schema_version: CACHE_SCHEMA_VERSION.into(),
                input_hash: input.input_hash.clone(),
                model_id: self.pin.snapshot.model_id.clone(),
                artifact_sha256: self.pin.snapshot.artifact_sha256.clone(),
                manifest_sha256: self.manifest_sha256.clone(),
                preprocessing_version: self.pin.snapshot.preprocessing_version.clone(),
                policy_version: self.pin.snapshot.policy_version.clone(),
                calibration_version: self.gates.version.clone(),
            }
        }
        fn receipt_key(&self, input: &ClaimedInput, classifier: &ClassifyInput) -> ReceiptKey {
            ReceiptKey {
                key_version: RECEIPT_KEY_VERSION.into(),
                occurrence: input.claim.occurrence.clone(),
                input_revision: input.claim.input_revision,
                source_revision: input.claim.source_revision.clone(),
                classifier_input_hash: classifier.input_hash.clone(),
                model_id: self.pin.snapshot.model_id.clone(),
                artifact_sha256: self.pin.snapshot.artifact_sha256.clone(),
                manifest_sha256: self.manifest_sha256.clone(),
                taxonomy_version: self.pin.snapshot.taxonomy_version.clone(),
                preprocessing_version: self.pin.snapshot.preprocessing_version.clone(),
                policy_version: self.pin.snapshot.policy_version.clone(),
                calibration_version: self.gates.version.clone(),
                receipt_schema_version: crate::classifier::RECEIPT_SCHEMA_VERSION.into(),
            }
        }
    }
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum ItemClassification {
        Evaluated {
            claim: AdmissionClaim,
            receipt: Box<StoredReceipt>,
        },
        Retryable {
            claim: AdmissionClaim,
            error: ClassifierError,
        },
    }

    struct InflightCancellation {
        cancel: Arc<std::sync::atomic::AtomicBool>,
        settled: bool,
    }
    impl Drop for InflightCancellation {
        fn drop(&mut self) {
            if !self.settled {
                self.cancel
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
    }
    async fn infer(
        context: &AdmissionContext,
        inputs: Vec<ClassifyInput>,
        control: &WorkControl,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        control.check_active()?;
        let service = context.pin.service.clone();
        let owned = control.clone();
        let mut cancellation = InflightCancellation {
            cancel: control.cancel.clone(),
            settled: false,
        };
        let mut task = tokio::task::spawn_blocking(move || {
            service.classify_batch_with_control(inputs, &owned)
        });
        loop {
            tokio::select! {
                result=&mut task => { cancellation.settled=true; return result.map_err(|_|ClassifierError::WorkerStopped)?; }
                _=tokio::time::sleep(Duration::from_millis(25)) => {
                    if let Err(error)=control.check_active() {
                        control.cancel.store(true,std::sync::atomic::Ordering::Release);
                        // Join the actual inference call before any caller releases owned claims.
                        let _=task.await; cancellation.settled=true; return Err(error);
                    }
                }
            }
        }
    }
    fn rejection_code(reason: crate::classifier::RejectionReason) -> &'static str {
        use crate::classifier::RejectionReason;
        match reason {
            RejectionReason::Blank => "blank",
            RejectionReason::UnsupportedLanguage => "unsupported_language",
            RejectionReason::WhollyOov => "wholly_oov",
            RejectionReason::InsufficientDomains => "insufficient_domains",
            RejectionReason::OutOfScope => "out_of_scope",
        }
    }
    fn operational_code(error: &ClassifierError) -> &'static str {
        match error {
            ClassifierError::Busy => "busy",
            ClassifierError::Timeout => "timeout",
            ClassifierError::Cancelled => "cancelled",
            ClassifierError::WorkerStopped => "worker_stopped",
            ClassifierError::RuntimeBudgetExceeded { .. } => "runtime_budget_exceeded",
            ClassifierError::Unavailable => "unavailable",
            ClassifierError::Incompatible(_) => "incompatible",
            _ => "invalid_classifier_result",
        }
    }
    /// Scores each distinct title/description once, then reapplies the real policy
    /// and reconstructs receipt/provenance for every occurrence, including cache hits.
    pub async fn classify_page(
        context: &AdmissionContext,
        inputs: Vec<ClaimedInput>,
        cache: &mut ComputationCache,
        control: &WorkControl,
    ) -> Result<PageClassification, AdmissionError> {
        if inputs.len() > crate::storage::classifier_admission::FOREGROUND_PAGE_ROWS {
            return Err(AdmissionError::InvalidPage {
                code: "page_row_limit".into(),
            });
        }
        control.check_active()?;
        let mut prepared = Vec::with_capacity(inputs.len());
        let mut misses = BTreeMap::new();
        let mut raw_by_key = BTreeMap::new();
        let mut failures = BTreeMap::new();
        let mut outcomes = Vec::with_capacity(inputs.len());
        let mut metrics = PageMetrics::default();
        for input in inputs {
            let classifier = ClassifyInput::new(
                input.claim.occurrence.record_id(),
                &input.article.title,
                &input.article.description,
                input.provenance.language.clone(),
            );
            if input.claim.occurrence.run_id != input.article.run_id
                || input.claim.occurrence.article_id != input.article.id
                || input.claim.input_revision == 0
                || input.claim.classifier_input_hash != classifier.input_hash
            {
                outcomes.push(ItemClassification::Retryable {
                    claim: input.claim,
                    error: ClassifierError::InvalidArtifact(
                        "staged_occurrence_identity_mismatch".into(),
                    ),
                });
                continue;
            }
            let key = context.computation_key(&classifier);
            // Same-page duplicates share a miss. Cache values contain no occurrence data.
            if !raw_by_key.contains_key(&key) && !misses.contains_key(&key) {
                if let Some(raw) = cache.get(&key) {
                    raw_by_key.insert(key.clone(), raw);
                } else {
                    // Runtime sample IDs are bounded independently of retained
                    // occurrence IDs. Keep the full occurrence on the final
                    // receipt and bind this raw-only request to its computation.
                    let mut wire = classifier.clone();
                    wire.occurrence_id = format!(
                        "atlas-raw-{}",
                        crate::classifier::artifact::sha256_hex(
                            codec::encode(&[
                                CACHE_SCHEMA_VERSION,
                                &key.model_id,
                                &key.artifact_sha256,
                                &key.manifest_sha256,
                                &key.input_hash
                            ])
                            .as_bytes()
                        )
                    );
                    misses.insert(key.clone(), wire);
                }
            }
            prepared.push((input, classifier, key));
        }
        let prepared_text: usize = prepared
            .iter()
            .map(|(input, classifier, _)| {
                input.article.title.capacity()
                    + input.article.description.capacity()
                    + classifier.title.capacity()
                    + classifier.description.capacity()
            })
            .sum();
        let missed_text: usize = misses
            .values()
            .map(|input| input.title.capacity() + input.description.capacity())
            .sum();
        let fixed_text = prepared_text
            .checked_add(missed_text)
            .ok_or(ClassifierError::CorpusTooLarge)?;
        metrics.owned_text_buffer_peak_bytes =
            u64::try_from(fixed_text).map_err(|_| ClassifierError::CorpusTooLarge)?;
        let misses: Vec<_> = misses.into_iter().collect();
        for batch in misses.chunks(crate::classifier::INFERENCE_BATCH_SIZE) {
            control.check_active()?;
            let inputs: Vec<ClassifyInput> = batch.iter().map(|(_, input)| input.clone()).collect();
            let dispatched_text: usize = inputs
                .iter()
                .map(|input| input.title.capacity() + input.description.capacity())
                .sum();
            let owned = fixed_text
                .checked_add(dispatched_text)
                .ok_or(ClassifierError::CorpusTooLarge)?;
            metrics.owned_text_buffer_peak_bytes = metrics
                .owned_text_buffer_peak_bytes
                .max(u64::try_from(owned).map_err(|_| ClassifierError::CorpusTooLarge)?);
            match infer(context, inputs, control).await {
                Ok(receipts) if receipts.len() == batch.len() => {
                    for ((key, input), receipt) in batch.iter().zip(receipts) {
                        if receipt.validate().is_err()
                            || receipt.input_hash != input.input_hash
                            || receipt.occurrence_id != input.occurrence_id
                            || receipt.model_id != context.pin.snapshot.model_id
                        {
                            failures.insert(
                                key.clone(),
                                ClassifierError::BridgeProtocol {
                                    operation: "classifyBatch".into(),
                                    code: "receipt_identity_mismatch".into(),
                                },
                            );
                            continue;
                        }
                        if let Some(diagnostics) = receipt.raw_diagnostics {
                            let computation = CachedComputation {
                                key: key.clone(),
                                raw: diagnostics.domains,
                            };
                            cache.insert(computation)?;
                            raw_by_key.insert(key.clone(), diagnostics.domains);
                            metrics.computations += 1;
                        } else {
                            failures.insert(
                                key.clone(),
                                ClassifierError::BridgeProtocol {
                                    operation: "classifyBatch".into(),
                                    code: "raw_diagnostics_missing".into(),
                                },
                            );
                        }
                    }
                }
                Ok(_) => {
                    for (key, _) in batch {
                        failures.insert(
                            key.clone(),
                            ClassifierError::BridgeProtocol {
                                operation: "classifyBatch".into(),
                                code: "receipt_count_mismatch".into(),
                            },
                        );
                    }
                }
                Err(error) => {
                    for (key, _) in batch {
                        failures.insert(key.clone(), error.clone());
                    }
                }
            }
        }
        let id = crate::classifier::artifact::ModelId::parse(&context.pin.snapshot.model_id)
            .ok_or_else(|| AdmissionError::InvalidReceipt {
                code: "invalid_pinned_model_id".into(),
            })?;
        for (input, classifier, key) in prepared {
            metrics.occurrences += 1;
            if let Some(error) = failures.get(&key) {
                *metrics
                    .operational_errors
                    .entry(operational_code(error).into())
                    .or_default() += 1;
                outcomes.push(ItemClassification::Retryable {
                    claim: input.claim,
                    error: error.clone(),
                });
                continue;
            }
            let raw = raw_by_key
                .get(&key)
                .ok_or_else(|| AdmissionError::InvalidReceipt {
                    code: "raw_computation_missing".into(),
                })?;
            let receipt = calibration::apply_gates(&classifier, raw, &id, &context.gates)?;
            if let Some(reason) = receipt.rejection_reason {
                *metrics
                    .rejections
                    .entry(rejection_code(reason).into())
                    .or_default() += 1;
            }
            let stored = StoredReceipt {
                key: context.receipt_key(&input, &classifier),
                receipt,
                provenance: input.provenance,
                publication_version: PUBLICATION_VERSION.into(),
            };
            stored.validate()?;
            outcomes.push(ItemClassification::Evaluated {
                claim: input.claim,
                receipt: Box::new(stored),
            });
        }
        metrics.cache = cache.metrics();
        Ok(PageClassification { outcomes, metrics })
    }
}
