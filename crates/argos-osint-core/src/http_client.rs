//! Process-wide pool of shared `reqwest` clients, keyed by their builder
//! settings.
//!
//! Building a `reqwest::Client` per request or per retry pays a full TCP + TLS
//! handshake every time because each client owns a private connection pool.
//! Every call site that used to build an identical client on each invocation
//! now goes through [`client`] with an explicit [`ClientKey`], so callers reuse
//! one connection pool per distinct configuration.
//!
//! [`ClientKey`] carries every setting that varies across the pooled call
//! sites (connect timeout, total request timeout, redirect policy). Two call
//! sites that share a key therefore receive a client that is byte-identical in
//! configuration to what each of them built before; no timeout or policy
//! changed. `Client::builder().build()` only constructs the client — it never
//! performs I/O — so building under the registry lock is safe and never spans
//! an `.await`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The complete set of builder settings that vary across pooled call sites.
///
/// Two call sites produce the same key (and therefore share one pooled client)
/// only when every field matches, so a pooled client is always configured
/// exactly like the fresh client the call site built before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct ClientKey {
    connect_timeout: Option<Duration>,
    timeout: Option<Duration>,
    no_redirect: bool,
}

impl ClientKey {
    /// Default settings: reqwest's own connect timeout, no total timeout and
    /// the default redirect-following policy.
    pub(crate) const fn new() -> Self {
        Self {
            connect_timeout: None,
            timeout: None,
            no_redirect: false,
        }
    }

    /// `.connect_timeout(...)`, as the per-attempt provider client builds.
    pub(crate) fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// `.timeout(...)`, the total per-request timeout.
    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// `.redirect(reqwest::redirect::Policy::none())`.
    pub(crate) fn no_redirect(mut self) -> Self {
        self.no_redirect = true;
        self
    }

    /// Rebuild the exact client this key describes. No I/O: this only
    /// constructs the client, so calling it under the registry lock is fine.
    fn build(&self) -> Result<reqwest::Client, reqwest::Error> {
        let mut builder = reqwest::Client::builder();
        if self.no_redirect {
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }
        if let Some(timeout) = self.connect_timeout {
            builder = builder.connect_timeout(timeout);
        }
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        builder.build()
    }
}

/// A client configured exactly by `key`, pooled process-wide per key.
///
/// Built on first use for each key and cloned afterwards (`reqwest::Client` is
/// Arc-backed, so cloning shares the connection pool). Returns
/// `reqwest::Error` so existing `.map_err(...)` / `.context(...)` call-site
/// error mapping stays identical.
pub(crate) fn client(key: ClientKey) -> Result<reqwest::Client, reqwest::Error> {
    static CLIENTS: OnceLock<Mutex<HashMap<ClientKey, reqwest::Client>>> = OnceLock::new();
    let registry = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    // A poisoned lock only means a panic while the map was mutably borrowed;
    // the clients already in the map are still valid, so recover them rather
    // than panicking (same poison-tolerant style as the rate-limit key cache).
    let mut clients = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(client) = clients.get(&key) {
        return Ok(client.clone());
    }
    let client = key.build()?;
    clients.insert(key, client.clone());
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pools_by_key_without_changing_configuration() {
        let decisions = ClientKey::new().timeout(Duration::from_secs(45));
        let first = client(decisions).expect("45s client");
        let second = client(decisions).expect("45s client");
        assert_eq!(format!("{first:?}"), format!("{second:?}"));

        // Different configurations must stay distinct pool entries.
        let attempt = client(ClientKey::new().connect_timeout(Duration::from_secs(5)))
            .expect("connect-timeout client");
        assert_ne!(format!("{first:?}"), format!("{attempt:?}"));

        // Redirect policy is part of the key, not a shared default.
        let holehe = ClientKey::new()
            .no_redirect()
            .timeout(Duration::from_secs(10));
        let no_redirect = client(holehe).expect("no-redirect client");
        let followed =
            client(ClientKey::new().timeout(Duration::from_secs(10))).expect("redirect client");
        assert_ne!(format!("{no_redirect:?}"), format!("{followed:?}"));
    }
}
