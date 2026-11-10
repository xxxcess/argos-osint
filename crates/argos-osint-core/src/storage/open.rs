//! The one way production code opens a SurrealDB store (P4).
//!
//! [`open`] connects, selects the namespace and database, and checks the
//! schema before handing out the client. Network runtime sessions verify the
//! exact schema without applying definitions. Bootstrap and legacy sessions
//! apply the version-guarded schema.

use std::fmt;
#[cfg(feature = "embedded")]
use std::path::PathBuf;
use std::time::Duration;

use surrealdb::engine::any::{connect, Any};
use surrealdb::opt::auth::{Database, Root};
use surrealdb::Surreal;
use url::Url;

use super::bootstrap::{self, Bootstrap};
use super::tx::TxError;

/// Where the store lives.
#[derive(Clone, PartialEq, Eq)]
pub enum Target {
    /// Shared per-home server on 127.0.0.1 (never a non-loopback address).
    Server {
        port: u16,
        username: String,
        password: String,
    },
    /// A server at an explicitly configured WebSocket origin.
    Network(NetworkTarget),
    /// Single-owner embedded RocksDB datastore directory.
    #[cfg(feature = "embedded")]
    Embedded { path: PathBuf },
}

/// The credential scope and schema authority for a network session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkAccess {
    /// Root credentials for the one-shot schema/bootstrap process.
    Bootstrap,
    /// Database-scoped credentials for API and worker processes.
    Runtime,
}

/// A validated network target. Construct it with [`Target::network`].
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkTarget {
    endpoint: Url,
    username: String,
    password: String,
    access: NetworkAccess,
}

impl Target {
    /// Select an explicitly configured SurrealDB WebSocket origin.
    ///
    /// Use `wss://` for endpoints outside the local or private service network.
    /// Credentials are passed separately and cannot appear in the URL.
    pub fn network(
        url: &str,
        user: String,
        password: String,
        access: NetworkAccess,
    ) -> Result<Self, TxError> {
        let invalid = || TxError::Abort("expected a ws:// or wss:// storage origin".into());
        if url.len() > 2048 || url.trim() != url || url.chars().any(char::is_control) {
            return Err(invalid());
        }
        let (_, authority) = url.split_once("://").ok_or_else(invalid)?;
        if authority
            .strip_suffix('/')
            .unwrap_or(authority)
            .contains(['/', '\\', '@', '?', '#'])
        {
            return Err(invalid());
        }
        let endpoint = Url::parse(url).map_err(|_| invalid())?;
        if !matches!(endpoint.scheme(), "ws" | "wss")
            || endpoint.host().is_none()
            || endpoint.port() == Some(0)
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(invalid());
        }
        if user.trim().is_empty()
            || user.len() > 128
            || user.chars().any(char::is_control)
            || password.is_empty()
            || password.len() > 4096
            || password.chars().any(char::is_control)
        {
            return Err(TxError::Abort("invalid network storage credentials".into()));
        }
        Ok(Self::Network(NetworkTarget {
            endpoint,
            username: user,
            password,
            access,
        }))
    }
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
            Target::Network(target) => f
                .debug_struct("Network")
                .field("endpoint", &target.endpoint)
                .field("access", &target.access)
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

/// Connect, select `ns`/`database`, and apply or verify the schema.
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
        Target::Network(target) => return open_network(target, ns, database).await,
        #[cfg(feature = "embedded")]
        Target::Embedded { path } => {
            let path = std::path::absolute(path)
                .map_err(|e| TxError::Abort(format!("embedded store path: {e}")))?;
            let url = format!("rocksdb://{}?versioned=false&sync=every", path.display());
            // The SDK has no explicit close: dropping the last client of a
            // datastore releases RocksDB's LOCK asynchronously, so a reopen
            // in the same process (reset, status after init) can briefly
            // see our own old handle (design.md 7b). Only that error is
            // waited out, bounded; another process is kept out earlier by
            // `embedded.lock`.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                match connect(url.as_str()).await {
                    Ok(db) => break db,
                    Err(e)
                        if e.to_string().contains("lock hold by current process")
                            && std::time::Instant::now() < deadline =>
                    {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    };
    db.use_ns(ns).use_db(database).await?;
    let bootstrap = bootstrap::ensure_schema(&db).await?;
    Ok(Opened { db, bootstrap })
}

async fn open_network(target: &NetworkTarget, ns: &str, database: &str) -> Result<Opened, TxError> {
    let connect = async {
        let db = connect_network(&target.endpoint).await?;
        match target.access {
            NetworkAccess::Bootstrap => {
                db.signin(Root {
                    username: target.username.clone(),
                    password: target.password.clone(),
                })
                .await
            }
            NetworkAccess::Runtime => {
                db.signin(Database {
                    namespace: ns.to_owned(),
                    database: database.to_owned(),
                    username: target.username.clone(),
                    password: target.password.clone(),
                })
                .await
            }
        }
        .map_err(|_| TxError::Abort("network storage authentication failed".into()))?;
        db.use_ns(ns)
            .use_db(database)
            .await
            .map_err(|_| TxError::Abort("could not select the network storage database".into()))?;
        Ok::<_, TxError>(db)
    };
    let db = tokio::time::timeout(Duration::from_secs(20), connect)
        .await
        .map_err(|_| TxError::Abort("network storage connection timed out".into()))??;
    let bootstrap = match target.access {
        NetworkAccess::Bootstrap => {
            let schema = bootstrap::ensure_schema(&db).await?;
            bootstrap::ensure_network_identity(&db).await?;
            schema
        }
        NetworkAccess::Runtime => bootstrap::verify_schema(&db).await?,
    };
    Ok(Opened { db, bootstrap })
}

#[cfg(feature = "server")]
async fn connect_network(endpoint: &Url) -> Result<Surreal<Any>, TxError> {
    let connection = if endpoint.scheme() == "wss" {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        // Other HTTP stacks may enable a different crypto provider. The
        // storage connection owns its provider without changing theirs.
        let tls = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .map_err(|_| TxError::Abort("network storage TLS configuration failed".into()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
        connect((endpoint.as_str(), surrealdb::opt::Config::new().rustls(tls))).await
    } else {
        connect(endpoint.as_str()).await
    };
    connection.map_err(|_| TxError::Abort("could not reach network storage".into()))
}

#[cfg(not(feature = "server"))]
async fn connect_network(_: &Url) -> Result<Surreal<Any>, TxError> {
    Err(TxError::Abort(
        "network storage requires the server feature".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{NetworkAccess, Target};

    #[test]
    fn network_targets_accept_only_websocket_origins_and_redact_credentials() {
        for url in [
            "ws://user:fixture-secret@db.internal",
            "ws://db.internal/?pass=fixture-secret",
            "wss://db.internal/path",
            "wss://db.internal/path/..",
            "wss://db.internal/#fixture-secret",
            "https://db.internal",
            "memory",
            "ws://db.internal:0",
        ] {
            let error = Target::network(
                url,
                "fixture-user".into(),
                "fixture-secret".into(),
                NetworkAccess::Runtime,
            )
            .unwrap_err();
            assert!(!error.to_string().contains("fixture-secret"), "{error}");
        }
        for url in ["ws://surrealdb:8000", "wss://db.example.test:8000"] {
            let target = Target::network(
                url,
                "fixture-user".into(),
                "fixture-secret".into(),
                NetworkAccess::Runtime,
            )
            .unwrap();
            let debug = format!("{target:?}");
            assert!(!debug.contains("fixture-user") && !debug.contains("fixture-secret"));
        }
    }
}
