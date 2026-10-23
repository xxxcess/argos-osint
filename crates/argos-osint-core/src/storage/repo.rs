//! The async SurrealDB repository (P4 consumer port, Option B).
//!
//! A [`Repository`] is a cheap-to-clone handle on one bootstrapped store.
//! Every constructor goes through `ensure_schema`. Consumers get one from
//! [`super::runtime::repository`] (the single process entry point) and call
//! the storage modules with [`Repository::db`]. The SQLite `Store` stays
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
                _hold: hold,
            }),
        }
    }

    /// The client, for the `storage::*` repository functions.
    pub fn db(&self) -> &Surreal<Any> {
        &self.inner.db
    }

    pub fn bootstrap(&self) -> Bootstrap {
        self.inner.bootstrap
    }

    /// Configured mode (None for explicit targets).
    pub fn mode(&self) -> Option<Mode> {
        self.inner.mode
    }
}
