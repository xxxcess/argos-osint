//! The async SurrealDB repository (P4 consumer port, Option B).
//!
//! A [`Repository`] is a cheap-to-clone handle on one bootstrapped store.
//! Every constructor goes through `ensure_schema`. Consumers get one from
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

struct Inner {
    db: Surreal<Any>,
    bootstrap: Bootstrap,
    mode: Option<Mode>,
    publisher: std::sync::OnceLock<super::publisher::Publisher>,
    actor_matcher: tokio::sync::OnceCell<Arc<crate::actor_catalog::Matcher>>,
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

    /// A store at an explicit target (tests, tools).
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
                actor_matcher: tokio::sync::OnceCell::new(),
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

    /// One deterministic actor matcher per open repository; a reset/reseed
    /// runs in a separate Venture child and therefore gets a fresh cache.
    pub(crate) async fn actor_matcher(&self) -> Result<Arc<crate::actor_catalog::Matcher>> {
        if let Some(matcher) = self.inner.actor_matcher.get() {
            return Ok(matcher.clone());
        }
        let repo = self.clone();
        // A request timeout must not discard catalog initialization and make
        // every subsequent request start the same expensive load again.
        tokio::spawn(async move {
            repo.inner
                .actor_matcher
                .get_or_try_init(|| async {
                    super::actor_catalog::load_matcher(&repo)
                        .await
                        .map(Arc::new)
                })
                .await
                .cloned()
        })
        .await
        .map_err(|error| anyhow!("actor matcher initialization worker: {error}"))?
    }

    /// Whether the shared actor catalog matcher has finished initialization.
    pub fn actor_matcher_ready(&self) -> bool {
        self.inner.actor_matcher.initialized()
    }

    pub fn bootstrap(&self) -> Bootstrap {
        self.inner.bootstrap
    }

    /// Configured mode (None for explicit targets).
    pub fn mode(&self) -> Option<Mode> {
        self.inner.mode
    }
}
