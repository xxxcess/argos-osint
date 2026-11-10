//! The async SurrealDB repository (P4 consumer port, Option B).
//!
//! A [`Repository`] is a cheap-to-clone handle on one bootstrapped store.
//! Every constructor applies or verifies the schema. Consumers get one from
//! [`super::runtime::repository`] (the single process entry point) and call
//! the storage modules with [`Repository::db`]. The legacy `Store` stays
//! untouched beside it until phase 5.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::bootstrap::{self, Bootstrap};
use super::home::{self, Hold, Layout, Mode};
use super::open::{self, Target};

/// The repository's cached catalog matcher: the served matcher with the
/// [`crate::actor_catalog::catalog_epoch`] it was built at. Reads clone the
/// `Arc` under a plain lock (no await); a stale epoch single-flights one
/// rebuild on a spawned worker (a request timeout must not discard the
/// rebuild) and swaps the served matcher atomically, so readers either keep
/// the old matcher or see the new one — never a partial build.
struct MatcherCache {
    served: std::sync::RwLock<Option<Arc<crate::actor_catalog::Matcher>>>,
    served_epoch: std::sync::atomic::AtomicU64,
    ready: std::sync::atomic::AtomicBool,
    rebuild: tokio::sync::Mutex<()>,
}

struct Inner {
    db: Surreal<Any>,
    bootstrap: Bootstrap,
    mode: Option<Mode>,
    publisher: std::sync::OnceLock<super::publisher::Publisher>,
    matcher: MatcherCache,
    _hold: Option<Hold>,
}

#[derive(Clone)]
pub struct Repository {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repository")
            .field("mode", &self.inner.mode)
            .field("bootstrap", &self.inner.bootstrap)
            .finish()
    }
}

impl Repository {
    /// This home's store in its configured mode (server lease or embedded
    /// lock held for the repository's lifetime).
    pub async fn open_home(layout: &Layout) -> Result<Self> {
        let store = home::open_home(layout).await?;
        Ok(Self::from_parts(
            store.opened.db,
            store.opened.bootstrap,
            Some(store.mode),
            Some(store.hold),
        ))
    }

    /// A store at an explicit target, with no local process ownership lease.
    pub async fn connect(target: &Target, ns: &str, database: &str) -> Result<Self> {
        let opened = open::open(target, ns, database)
            .await
            .map_err(|e| anyhow!("open store: {e}"))?;
        Ok(Self::from_parts(opened.db, opened.bootstrap, None, None))
    }

    /// Adopt an already connected client (the embedded engine allows one
    /// datastore per process, so a caller may share it). Still selects
    /// `ns`/`database` and runs `ensure_schema`.
    pub async fn attach(client: Surreal<Any>, ns: &str, database: &str) -> Result<Self> {
        client.use_ns(ns).use_db(database).await?;
        let bootstrap = bootstrap::ensure_schema(&client)
            .await
            .map_err(|e| anyhow!("bootstrap store: {e}"))?;
        Ok(Self::from_parts(client, bootstrap, None, None))
    }

    fn from_parts(
        db: Surreal<Any>,
        bootstrap: Bootstrap,
        mode: Option<Mode>,
        hold: Option<Hold>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                bootstrap,
                mode,
                publisher: std::sync::OnceLock::new(),
                matcher: MatcherCache {
                    served: std::sync::RwLock::new(None),
                    served_epoch: std::sync::atomic::AtomicU64::new(u64::MAX),
                    ready: std::sync::atomic::AtomicBool::new(false),
                    rebuild: tokio::sync::Mutex::new(()),
                },
                _hold: hold,
            }),
        }
    }

    /// The client, for the `storage::*` repository functions.
    pub fn db(&self) -> &Surreal<Any> {
        &self.inner.db
    }

    /// The group-commit publisher for memory writes, started on first use
    /// (must be called inside the runtime).
    pub fn publisher(&self) -> &super::publisher::Publisher {
        self.inner
            .publisher
            .get_or_init(|| super::publisher::Publisher::spawn(self.inner.db.clone()))
    }

    /// One deterministic actor matcher per open repository, rebuilt
    /// single-flight when a catalog mutation bumped the epoch. A reset/reseed
    /// runs in a separate Venture child and therefore still gets a fresh
    /// cache (its epoch bumps are not observed by this process).
    pub async fn actor_matcher(&self) -> Result<Arc<crate::actor_catalog::Matcher>> {
        if let Some(matched) = self.served_matcher() {
            return Ok(matched);
        }
        let repo = self.clone();
        // The rebuild runs on a spawned worker and holds the rebuild lock for
        // its whole duration: a request timeout must neither discard catalog
        // initialization nor start the same expensive load twice.
        tokio::spawn(async move {
            let _guard = repo.inner.matcher.rebuild.lock().await;
            if let Some(matched) = repo.served_matcher() {
                return Ok(matched);
            }
            let built: Arc<crate::actor_catalog::Matcher> =
                Arc::new(super::actor_catalog::load_matcher(&repo).await?);
            // Atomic generation swap: readers see the old matcher or this one.
            *repo
                .inner
                .matcher
                .served
                .write()
                .expect("matcher cache lock poisoned") = Some(built.clone());
            repo.inner.matcher.served_epoch.store(
                crate::actor_catalog::catalog_epoch(),
                std::sync::atomic::Ordering::Release,
            );
            repo.inner
                .matcher
                .ready
                .store(true, std::sync::atomic::Ordering::Release);
            Ok(built)
        })
        .await
        .map_err(|error| anyhow!("actor matcher initialization worker: {error}"))?
    }

    /// The served matcher when it is current for this process's epoch.
    fn served_matcher(&self) -> Option<Arc<crate::actor_catalog::Matcher>> {
        let served = self
            .inner
            .matcher
            .served
            .read()
            .expect("matcher cache lock poisoned")
            .clone();
        let current = self
            .inner
            .matcher
            .served_epoch
            .load(std::sync::atomic::Ordering::Acquire)
            == crate::actor_catalog::catalog_epoch();
        served.filter(|_| current)
    }

    /// Whether the shared actor catalog matcher has finished initialization.
    pub fn actor_matcher_ready(&self) -> bool {
        self.inner
            .matcher
            .ready
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn bootstrap(&self) -> Bootstrap {
        self.inner.bootstrap
    }

    /// Configured mode (None for explicit targets).
    pub fn mode(&self) -> Option<Mode> {
        self.inner.mode
    }
}
