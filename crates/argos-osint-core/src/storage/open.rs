//! The one way production code opens a SurrealDB store (P4).
//!
//! [`open`] connects (loopback server with root sign-in, or the embedded
//! RocksDB engine), selects the namespace and database, and runs
//! [`bootstrap::ensure_schema`] before handing out the client, so no real
//! path ever sees a database without its schema. A database written by a
//! newer build is refused here, before any caller reads it.

use std::fmt;
#[cfg(feature = "embedded")]
use std::path::PathBuf;

use surrealdb::engine::any::{connect, Any};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

use super::bootstrap::{self, Bootstrap};
use super::tx::TxError;

/// Where the store lives.
#[derive(Clone)]
pub enum Target {
    /// Shared per-home server on 127.0.0.1 (never a non-loopback address).
    Server {
        port: u16,
        username: String,
        password: String,
    },
    /// Single-owner embedded RocksDB datastore directory.
    #[cfg(feature = "embedded")]
    Embedded { path: PathBuf },
}

impl fmt::Debug for Target {
    // Never print the password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Server { port, username, .. } => f
                .debug_struct("Server")
                .field("port", port)
                .field("username", username)
                .field("password", &"<redacted>")
                .finish(),
            #[cfg(feature = "embedded")]
            Target::Embedded { path } => f.debug_struct("Embedded").field("path", path).finish(),
        }
    }
}

/// An open store whose schema is in place.
#[derive(Clone)]
pub struct Opened {
    pub db: Surreal<Any>,
    pub bootstrap: Bootstrap,
}

/// Connect, select `ns`/`database`, and ensure the schema.
pub async fn open(target: &Target, ns: &str, database: &str) -> Result<Opened, TxError> {
    let db = match target {
        Target::Server {
            port,
            username,
            password,
        } => {
            let db = connect(format!("ws://127.0.0.1:{port}")).await?;
            db.signin(Root {
                username: username.clone(),
                password: password.clone(),
            })
            .await?;
            db
        }
        #[cfg(feature = "embedded")]
        Target::Embedded { path } => {
            let path = std::path::absolute(path)
                .map_err(|e| TxError::Abort(format!("embedded store path: {e}")))?;
            connect(format!(
                "rocksdb://{}?versioned=false&sync=every",
                path.display()
            ))
            .await?
        }
    };
    db.use_ns(ns).use_db(database).await?;
    let bootstrap = bootstrap::ensure_schema(&db).await?;
    Ok(Opened { db, bootstrap })
}
