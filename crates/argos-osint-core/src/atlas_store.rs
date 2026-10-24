//! Atlas storage dispatch (SurrealDB migration, Atlas switch).
//!
//! Atlas callers (`atlas`, `atlas_metadata`, `atlas_insights`,
//! `atlas_memory`) talk to [`AtlasStore`]: SQLite (`Store`) unless the
//! process selected SurrealDB (`storage::runtime::configured`), in which
//! case every call goes to the repository and SQLite is never opened. The
//! two stores never both serve a process.
//!
//! The SQLite store is not `Sync`, so it sits behind a mutex held only
//! inside one synchronous call; futures that hold an [`AtlasStore`] stay
//! `Send`. Operations with no SurrealDB port fail with "… is not on
//! SurrealDB yet" instead of silently touching SQLite.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{anyhow, Result};

use crate::atlas_metadata::ArticleMetadata;
use crate::store::{AtlasArticleRow, AtlasRunRow, Store};

pub use crate::recon::backend::surreal_selected;

/// The Atlas store a caller uses for one unit of work.
pub enum AtlasStore {
    Sqlite(Mutex<Store>),
    #[cfg(feature = "surreal")]
    Surreal(crate::storage::repo::Repository),
}

/// Error for an operation that is still SQLite-only.
pub fn not_ported(op: &str) -> anyhow::Error {
    anyhow!("{op} is not on SurrealDB yet")
}

#[cfg(feature = "surreal")]
fn tx(op: &'static str) -> impl FnOnce(crate::storage::tx::TxError) -> anyhow::Error {
    move |e| anyhow!("{op}: {e}")
}

macro_rules! dispatch {
    ($self:ident, $s:ident => $sqlite:expr, $r:ident => $surreal:expr) => {
        match $self {
            AtlasStore::Sqlite(m) => {
                let $s = lock(m);
                $sqlite
            }
            #[cfg(feature = "surreal")]
            AtlasStore::Surreal($r) => $surreal,
        }
    };
}

fn lock(m: &Mutex<Store>) -> MutexGuard<'_, Store> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AtlasStore {
    /// The selected store: the process repository under SurrealDB,
    /// otherwise `Store::open(db_path)`.
    pub async fn open(db_path: &Path) -> Result<Self> {
        #[cfg(feature = "surreal")]
        if surreal_selected() {
            return Ok(Self::Surreal(crate::storage::runtime::repository().await?));
        }
        Ok(Self::Sqlite(Mutex::new(Store::open(db_path)?)))
    }

    pub fn from_sqlite(store: Store) -> Self {
        Self::Sqlite(Mutex::new(store))
    }

    #[cfg(feature = "surreal")]
    pub fn from_repository(repo: crate::storage::repo::Repository) -> Self {
        Self::Surreal(repo)
    }

    pub fn is_surreal(&self) -> bool {
        !matches!(self, Self::Sqlite(_))
    }

    /// Run `f` on the SQLite store, or fail with `op` not ported.
    pub fn sqlite<T>(&self, op: &str, f: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        match self {
            Self::Sqlite(m) => {
                let _ = op;
                f(&mut lock(m))
            }
            #[cfg(feature = "surreal")]
            Self::Surreal(_) => Err(not_ported(op)),
        }
    }

    /// The repository under SurrealDB.
    #[cfg(feature = "surreal")]
    pub fn repository(&self) -> Option<&crate::storage::repo::Repository> {
        match self {
            Self::Surreal(r) => Some(r),
            Self::Sqlite(_) => None,
        }
    }

    // ---- runs -------------------------------------------------------------

    /// Latest publication receipt of a run (memory phase).
    pub async fn publication_receipt(
        &self,
        run_id: &str,
    ) -> Result<Option<crate::store::PublicationReceipt>> {
        match self {
            Self::Sqlite(m) => lock(m).atlas_publication_receipt(run_id),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::atlas_memory::publication_receipt(r.db(), run_id).await
            }
        }
    }

    /// Phase-completion check of one publication receipt.
    pub async fn verify_publication(
        &self,
        receipt: &crate::store::PublicationReceipt,
    ) -> Result<crate::store::PublicationVerification> {
        match self {
            Self::Sqlite(m) => lock(m).verify_atlas_publication(receipt),
            #[cfg(feature = "surreal")]
            Self::Surreal(r) => {
                crate::storage::atlas_memory::verify_publication(r.db(), receipt).await
            }
        }
    }

    pub async fn list_runs(&self) -> Result<Vec<AtlasRunRow>> {
        dispatch!(self, s => s.atlas_list_runs(),
            r => crate::storage::atlas::list_runs(r.db()).await.map_err(tx("atlas runs")))
    }
    pub async fn latest_run(&self) -> Result<Option<AtlasRunRow>> {
        dispatch!(self, s => s.atlas_latest_run(),
            r => crate::storage::atlas::latest_run(r.db()).await.map_err(tx("atlas latest run")))
    }
    pub async fn insert_run(&self, id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
        dispatch!(self, s => s.atlas_insert_run(id, cursor_json, stats_json),
            r => crate::storage::atlas::insert_run(r.db(), id, cursor_json, stats_json)
                .await
                .map_err(tx("atlas insert run")))
    }
    pub async fn save(&self, id: &str, cursor_json: &str, stats_json: &str) -> Result<()> {
        dispatch!(self, s => s.atlas_save(id, cursor_json, stats_json),
            r => crate::storage::atlas::save(r.db(), id, cursor_json, stats_json)
                .await
                .map_err(tx("atlas save")))
    }
    /// Cursor, stats and a page receipt in one transaction.
    pub async fn save_with_receipt(
        &self,
        id: &str,
        receipt_key: &str,
        receipt_json: &str,
        cursor_json: &str,
        stats_json: &str,
    ) -> Result<()> {
        dispatch!(self, s => {
            let tx = s.conn.unchecked_transaction()?;
            s.app_state_set(receipt_key, receipt_json)?;
            s.atlas_save(id, cursor_json, stats_json)?;
            tx.commit()?;
            Ok(())
        }, r => crate::storage::atlas::save_with_receipt(
                r.db(), id, receipt_key, receipt_json, cursor_json, stats_json,
            )
            .await
            .map_err(tx("atlas receipt")))
    }
    pub async fn set_state(&self, id: &str, state: &str, note: &str, finished: bool) -> Result<()> {
        dispatch!(self, s => s.atlas_set_state(id, state, note, finished),
            r => crate::storage::atlas::set_state(r.db(), id, state, note, finished)
                .await
                .map_err(tx("atlas set state")))
    }

    // ---- articles ---------------------------------------------------------

    pub async fn list_articles(&self, run_id: &str) -> Result<Vec<AtlasArticleRow>> {
        dispatch!(self, s => s.atlas_list_articles(run_id),
            r => crate::storage::atlas::list_articles(r.db(), run_id)
                .await
                .map_err(tx("atlas articles")))
    }
    pub async fn recent_articles(&self) -> Result<Vec<AtlasArticleRow>> {
        dispatch!(self, s => s.atlas_recent_articles(),
            r => crate::storage::atlas::recent_articles(r.db())
                .await
                .map_err(tx("atlas recent articles")))
    }
    pub async fn article(&self, run_id: &str, article_id: &str) -> Result<Option<AtlasArticleRow>> {
        if run_id.is_empty() || article_id.is_empty() {
            return Ok(None);
        }
        dispatch!(self, s => s.atlas_article(run_id, article_id),
            r => crate::storage::atlas::article(r.db(), run_id, article_id)
                .await
                .map_err(tx("atlas article")))
    }
    pub async fn upsert_article(&self, row: &AtlasArticleRow) -> Result<()> {
        dispatch!(self, s => s.atlas_upsert_article(row),
            r => crate::storage::atlas::upsert_article(r.db(), row)
                .await
                .map_err(tx("atlas upsert article")))
    }
    /// One article row of one run, then the article and lifecycle reconciles.
    pub async fn delete_article(&self, run_id: &str, article_id: &str) -> Result<()> {
        dispatch!(self, s => s.atlas_delete_article(run_id, article_id),
            r => crate::storage::atlas_delete::delete_article(r.publisher(), run_id, article_id)
                .await
                .map_err(tx("atlas delete article")))
    }

    // ---- metadata ---------------------------------------------------------

    pub async fn metadata(
        &self,
        run_id: &str,
        article_id: &str,
    ) -> Result<Option<ArticleMetadata>> {
        dispatch!(self, s => s.atlas_metadata(run_id, article_id),
            r => crate::storage::atlas_metadata::metadata(r.db(), run_id, article_id)
                .await
                .map_err(tx("atlas metadata")))
    }
    pub async fn save_metadata(&self, metadata: &ArticleMetadata) -> Result<bool> {
        dispatch!(self, s => s.atlas_save_metadata(metadata),
            r => crate::storage::atlas_metadata::save_metadata(r.db(), metadata)
                .await
                .map_err(tx("atlas save metadata")))
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn set_classification(
        &self,
        run_id: &str,
        article_id: &str,
        captured_hash: &str,
        status: &str,
        receipt_json: &str,
        category: Option<&str>,
    ) -> Result<bool> {
        dispatch!(self, s => s.atlas_set_classification(
                run_id, article_id, captured_hash, status, receipt_json, category),
            r => crate::storage::atlas_metadata::set_classification(
                    r.db(), run_id, article_id, captured_hash, status, receipt_json, category)
                .await
                .map_err(tx("atlas classification")))
    }

    // ---- quota and app state ---------------------------------------------

    pub async fn quota_used(&self, provider: &str, day: &str) -> Result<u32> {
        dispatch!(self, s => s.atlas_quota_used(provider, day),
            r => crate::storage::atlas::quota_used(r.db(), provider, day)
                .await
                .map_err(tx("atlas quota")))
    }
    pub async fn quota_bump(&self, provider: &str, day: &str) -> Result<u32> {
        dispatch!(self, s => s.atlas_quota_bump(provider, day),
            r => crate::storage::atlas::quota_bump(r.db(), provider, day)
                .await
                .map_err(tx("atlas quota bump")))
    }
    pub async fn app_state_get(&self, key: &str) -> Result<Option<String>> {
        if key.is_empty() {
            return Ok(None);
        }
        dispatch!(self, s => s.app_state_get(key),
            r => crate::storage::pipeline_inputs::app_state_get(r, key).await)
    }
    pub async fn app_state_set(&self, key: &str, value: &str) -> Result<()> {
        if key.is_empty() {
            return Err(anyhow!("app_state: empty key"));
        }
        dispatch!(self, s => s.app_state_set(key, value),
            r => crate::storage::pipeline_inputs::app_state_set(r, key, value).await)
    }
    pub async fn app_state_delete(&self, key: &str) -> Result<()> {
        dispatch!(self, s => {
            s.conn.execute("DELETE FROM app_state WHERE key=?1", [key])?;
            Ok(())
        }, r => crate::storage::pipeline_inputs::app_state_delete(r, key).await)
    }
    pub async fn tool_enabled(&self, id: &str) -> Result<bool> {
        dispatch!(self, s => s.tool_enabled(id),
            r => crate::storage::recon_threads::tool_enabled(r, id).await)
    }

    // ---- work units -------------------------------------------------------

    pub async fn save_unit_manifest(&self, m: &crate::atlas_work::UnitManifest) -> Result<()> {
        dispatch!(self, s => s.atlas_save_unit_manifest(m),
            r => crate::storage::atlas_work::save_unit_manifest(r.db(), m).await)
    }
    pub async fn unit_manifests(
        &self,
        run_id: &str,
        stage: i32,
    ) -> Result<Vec<crate::atlas_work::UnitManifest>> {
        dispatch!(self, s => s.atlas_get_unit_manifests(run_id, stage),
            r => crate::storage::atlas_work::unit_manifests(r.db(), run_id, stage).await)
    }
    pub async fn all_unit_manifests(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::atlas_work::UnitManifest>> {
        dispatch!(self, s => s.atlas_get_all_unit_manifests(run_id),
            r => crate::storage::atlas_work::all_unit_manifests(r.db(), run_id).await)
    }
}
