//! Current native publication visibility while lifecycle cleanup is pending.
//! Operational stamps never grant authority to curated/user relationships.
use super::classifier_admission::{self as admission, NativeActorGeneration};
use super::tx::{Tx, TxError};
use crate::atlas_classify::OccurrenceKey;
use crate::memory_metadata::{EndpointRef, MemoryPayload};
use std::collections::BTreeMap;

pub(crate) struct NativeVisibility {
    tx: Tx,
    actors: BTreeMap<OccurrenceKey, Option<NativeActorGeneration>>,
    related: BTreeMap<OccurrenceKey, Option<String>>,
}
impl NativeVisibility {
    pub(crate) fn new(tx: Tx) -> Self {
        Self {
            tx,
            actors: BTreeMap::new(),
            related: BTreeMap::new(),
        }
    }
    pub(crate) async fn prefetch(&mut self, keys: &[OccurrenceKey]) -> Result<(), TxError> {
        let keys: Vec<_> = keys
            .iter()
            .filter(|key| !self.actors.contains_key(*key))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for page in keys.chunks(64) {
            let mut generations =
                admission::native_actor_generations_many_in(&self.tx, page).await?;
            for key in page {
                self.actors.insert(key.clone(), generations.remove(key));
            }
        }
        Ok(())
    }
    pub(crate) async fn mention_current(
        &mut self,
        key: &OccurrenceKey,
        revision: &str,
        batch: &str,
    ) -> Result<bool, TxError> {
        if revision.is_empty() || batch.is_empty() {
            return Ok(false);
        }
        self.prefetch(std::slice::from_ref(key)).await?;
        Ok(self
            .actors
            .get(key)
            .and_then(Option::as_ref)
            .is_some_and(|current| {
                current.input_revision == revision && current.publication_batch == batch
            }))
    }
    async fn batch_current(&mut self, key: &OccurrenceKey, batch: &str) -> Result<bool, TxError> {
        if batch.is_empty() {
            return Ok(false);
        }
        self.prefetch(std::slice::from_ref(key)).await?;
        Ok(self
            .actors
            .get(key)
            .and_then(Option::as_ref)
            .is_some_and(|current| current.publication_batch == batch))
    }
    async fn related_revision(&mut self, key: &OccurrenceKey) -> Result<Option<String>, TxError> {
        if !self.related.contains_key(key) {
            let mut revisions =
                admission::native_related_revisions_many_in(&self.tx, std::slice::from_ref(key))
                    .await?;
            self.related.insert(key.clone(), revisions.remove(key));
        }
        Ok(self.related.get(key).cloned().flatten())
    }
    /// Only records carrying the existing native ingest ownership receipt are
    /// filtered here. Validation-bearing links and independent evidence stay
    /// under their existing authority, regardless of this worker's generation.
    pub(crate) async fn memory_current(
        &mut self,
        memory_id: &str,
        payload: &MemoryPayload,
        batch: &str,
    ) -> Result<bool, TxError> {
        match payload {
            MemoryPayload::Link(link)
                if link.validation.is_none()
                    && link.relation == "mentioned_in"
                    && link
                        .receipts
                        .iter()
                        .any(|receipt| receipt.starts_with("atlas_article_mentions:")) =>
            {
                if !super::lifecycle::ingest_memory_writable_in(&self.tx, memory_id).await? {
                    return Ok(true);
                }
                let EndpointRef::Article {
                    run_id, article_id, ..
                } = &link.target
                else {
                    return Ok(false);
                };
                self.batch_current(&OccurrenceKey::new(run_id, article_id), batch)
                    .await
            }
            MemoryPayload::Related(related)
                if related
                    .source_revisions
                    .iter()
                    .any(|revision| revision.starts_with("ingest:")) =>
            {
                if !super::lifecycle::ingest_memory_writable_in(&self.tx, memory_id).await? {
                    return Ok(true);
                }
                let mut owner_current = false;
                for endpoint in [&related.source, &related.target] {
                    let EndpointRef::Article {
                        run_id, article_id, ..
                    } = endpoint
                    else {
                        return Ok(false);
                    };
                    let key = OccurrenceKey::new(run_id, article_id);
                    let Some(revision) = self.related_revision(&key).await? else {
                        return Ok(false);
                    };
                    let tag =
                        super::codec::encode(&["native-related-v1", &key.record_id(), &revision]);
                    if !related.source_revisions.contains(&tag) {
                        return Ok(false);
                    }
                    if related.source_revisions.iter().any(|receipt| {
                        receipt.starts_with(&format!("ingest:{run_id}/{article_id}:"))
                    }) && self.batch_current(&key, batch).await?
                    {
                        owner_current = true;
                    }
                }
                Ok(owner_current)
            }
            _ => Ok(true),
        }
    }
}
