//! Process-wide pacing at the model HTTP boundary, shared by every caller.
//!
//! Each provider has one FIFO admission queue across models, accounts and
//! transports. A send advances the clock from *now*, so idle time never earns
//! a burst. Dropping a waiting future removes it without reserving a send.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::time::Instant;

use crate::secrets::ProviderSecret;

pub(crate) const PROVIDER_RPM: [(&str, u32); 3] =
    [("openrouter", 20), ("google", 10), ("nvidia", 40)];

struct Pacer {
    queue: tokio::sync::Mutex<()>,
    not_before: Mutex<Option<Instant>>,
    interval: Duration,
}

impl Pacer {
    fn new(rpm: u32) -> Self {
        Self {
            queue: tokio::sync::Mutex::new(()),
            not_before: Mutex::new(None),
            interval: Duration::from_secs(60) / rpm,
        }
    }

    async fn wait(&self) {
        let _turn = self.queue.lock().await;
        loop {
            let deadline = {
                let mut next = self.not_before.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                match *next {
                    Some(at) if at > now => Some(at),
                    _ => {
                        *next = Some(now + self.interval);
                        None
                    }
                }
            };
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => return,
            }
            // Another request may have extended the provider cooldown while
            // this caller slept. Recheck under the same clock lock.
        }
    }

    fn defer(&self, delay: Duration) {
        let mut next = self.not_before.lock().unwrap_or_else(|e| e.into_inner());
        let until = Instant::now() + delay;
        *next = Some(next.map_or(until, |at| at.max(until)));
    }
}

fn pacer(secret: &ProviderSecret) -> Option<&'static Pacer> {
    // The binary's tests link core as a regular library, with `fixtures`
    // enabled. Their independent loopback servers must not inherit a live
    // provider clock or another fixture's scripted 429/503 cooldown.
    #[cfg(feature = "fixtures")]
    if url::Url::parse(&secret.base_url)
        .ok()
        .and_then(|u| u.host().map(|host| host.to_owned()))
        .is_some_and(|host| match host {
            url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
            url::Host::Ipv4(ip) => ip.is_loopback(),
            url::Host::Ipv6(ip) => ip.is_loopback(),
        })
    {
        return None;
    }
    static PACERS: OnceLock<[Pacer; 3]> = OnceLock::new();
    let kind = crate::provider::effective_kind(secret);
    let i = PROVIDER_RPM.iter().position(|(p, _)| *p == kind)?;
    Some(&PACERS.get_or_init(|| PROVIDER_RPM.map(|(_, rpm)| Pacer::new(rpm)))[i])
}

// Mock transports in other unit tests do not share live provider quota state
// across independent Tokio runtimes. Transport coverage below opts into a
// real, isolated pacer. Builds without `fixtures` pace custom endpoints too.
#[cfg(test)]
tokio::task_local! {
    static TEST_PACER: std::sync::Arc<Pacer>;
}

/// Wait immediately before sending, after compilation and authorization.
/// No request deadline or retry sleep includes this queue residence.
pub(crate) async fn wait(secret: &ProviderSecret) {
    #[cfg(test)]
    {
        if let Ok(pacer) = TEST_PACER.try_with(Clone::clone) {
            pacer.wait().await;
        }
        let _ = secret;
    }
    #[cfg(not(test))]
    if let Some(pacer) = pacer(secret) {
        pacer.wait().await;
    }
}

fn defer(secret: &ProviderSecret, delay: Duration) {
    #[cfg(test)]
    {
        let _ = TEST_PACER.try_with(|pacer| pacer.defer(delay));
        let _ = secret;
    }
    #[cfg(not(test))]
    if let Some(pacer) = pacer(secret) {
        pacer.defer(delay);
    }
}

/// Apply feedback even on the last attempt or when switching fallback routes.
/// Retry ownership stays with provider_chain; this only delays future sends.
pub(crate) fn response(secret: &ProviderSecret, status: u16, headers: &reqwest::header::HeaderMap) {
    if status < 400 {
        return;
    }
    let delay = crate::provider_diag::retry_after_uncapped(headers).or(match status {
        429 => Some(Duration::from_secs(60)),
        503 => Some(Duration::from_secs(10)),
        _ => None,
    });
    if let Some(delay) = delay {
        defer(secret, delay);
    }
}

pub(crate) fn failure(secret: &ProviderSecret, failure: &crate::provider_diag::ProviderFailure) {
    // HTTP errors already update the clock at response headers. This covers
    // rate limits and overloads delivered inside an HTTP-200 event stream.
    if failure.http_status.is_some_and(|s| s >= 400) {
        return;
    }
    use crate::provider_diag::Category;
    let delay = match failure.category {
        Category::RateLimited => Some(Duration::from_secs(60)),
        Category::Server | Category::SseError => Some(Duration::from_secs(10)),
        _ => None,
    };
    if let Some(delay) = failure.retry_after_ms.map(Duration::from_millis).or(delay) {
        defer(secret, delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn provider_scope_ignores_model_account_and_endpoint() {
        let a = crate::provider_attempt::mock::secret("https://openrouter.ai/api/v1");
        let mut b = a.clone();
        b.model = "other-model".into();
        b.base_url = "https://other-proxy.example/v1".into();
        assert!(std::ptr::eq(pacer(&a).unwrap(), pacer(&b).unwrap()));
        b.kind = "google".into();
        assert!(!std::ptr::eq(pacer(&a).unwrap(), pacer(&b).unwrap()));
        b.kind = "local".into();
        assert!(pacer(&b).is_none());
    }

    #[cfg(feature = "fixtures")]
    #[test]
    fn binary_fixture_servers_do_not_share_production_cooldowns() {
        for base in [
            "http://127.0.0.1:1234/v1",
            "http://[::1]:1234/v1",
            "http://localhost:1234/v1",
        ] {
            let secret = crate::provider_attempt::mock::secret(base);
            assert!(pacer(&secret).is_none());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn providers_are_spaced_without_bursts_or_catch_up() {
        for (_, rpm) in PROVIDER_RPM {
            let pacer = Pacer::new(rpm);
            let start = Instant::now();
            for n in 0..=rpm {
                pacer.wait().await;
                assert_eq!(start.elapsed(), pacer.interval * n);
            }
            assert_eq!(start.elapsed(), Duration::from_secs(60));
            tokio::time::sleep(Duration::from_secs(120)).await;
            pacer.wait().await;
            let sent = Instant::now();
            pacer.wait().await;
            assert_eq!(sent.elapsed(), pacer.interval);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fifo_queue_cancellation_does_not_reserve_capacity() {
        let pacer = Arc::new(Pacer::new(20));
        pacer.wait().await;
        let cancelled = tokio::spawn({
            let pacer = pacer.clone();
            async move { pacer.wait().await }
        });
        tokio::task::yield_now().await;
        let mut tasks = Vec::new();
        for i in 0..3 {
            tasks.push(tokio::spawn({
                let pacer = pacer.clone();
                async move {
                    pacer.wait().await;
                    (i, Instant::now())
                }
            }));
            tokio::task::yield_now().await;
        }
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        let start = Instant::now();
        for (i, task) in tasks.into_iter().enumerate() {
            let (order, at) = task.await.unwrap();
            assert_eq!(order, i);
            assert_eq!(at - start, Duration::from_secs(3 * (i as u64 + 1)));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cooldown_rechecks_sleepers_and_does_not_block_other_providers() {
        let router = Arc::new(Pacer::new(20));
        let google = Pacer::new(10);
        router.wait().await;
        let start = Instant::now();
        let queued = tokio::spawn({
            let router = router.clone();
            async move {
                router.wait().await;
                Instant::now()
            }
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        router.defer(Duration::from_secs(30));
        router.defer(Duration::from_secs(2));
        google.wait().await;
        assert_eq!(start.elapsed(), Duration::from_secs(1));
        assert_eq!(queued.await.unwrap() - start, Duration::from_secs(31));
        router.wait().await;
        assert_eq!(start.elapsed(), Duration::from_secs(34));
    }

    #[tokio::test(start_paused = true)]
    async fn feedback_honors_retry_after_and_default_rate_limit_wait() {
        let pacer = Arc::new(Pacer::new(20));
        TEST_PACER
            .scope(pacer.clone(), async {
                let secret = crate::provider_attempt::mock::secret("https://openrouter.ai/api/v1");
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("retry-after", "600".parse().unwrap());
                response(&secret, 429, &headers);
                let start = Instant::now();
                pacer.wait().await;
                assert_eq!(start.elapsed(), Duration::from_secs(600));
                response(&secret, 429, &reqwest::header::HeaderMap::new());
                pacer.wait().await;
                assert_eq!(start.elapsed(), Duration::from_secs(660));
                response(&secret, 503, &reqwest::header::HeaderMap::new());
                pacer.wait().await;
                assert_eq!(start.elapsed(), Duration::from_secs(670));
            })
            .await;
    }

    #[tokio::test]
    async fn all_completion_and_decision_transports_share_the_pacer() {
        use crate::provider::{self, ChatMessage};
        use crate::provider_attempt::{self, mock, Deadlines, Transport};
        let server = mock::serve(vec![
            mock::ok_json("one", "stop"),
            mock::Reply::Sse(vec![
                mock::delta("two"),
                mock::finish("stop"),
                "data: [DONE]\n\n".into(),
            ]),
            mock::ok_json("three", "stop"),
            mock::Reply::Json(200, r#"{"answers":{}}"#.into()),
            mock::Reply::Json(200, r#"{"text":"transcribed"}"#.into()),
        ])
        .await;
        let secret = crate::secrets::ProviderSecret {
            kind: "openrouter".into(),
            base_url: server.base_url.clone(),
            model: "nvidia/nemotron-3-super-120b-a12b:free".into(),
            api_key: Some("fixture-key".into()),
            ..mock::secret(&server.base_url)
        };
        let messages = [ChatMessage {
            role: "user".into(),
            content: "hello".into(),
            tool_call_id: None,
            tool_calls: vec![],
        }];
        let mut pacer = Pacer::new(20);
        pacer.interval = Duration::from_millis(30);
        let pacer = Arc::new(pacer);
        TEST_PACER
            .scope(pacer.clone(), async {
                provider::complete_one(&secret, &messages, &[])
                    .await
                    .unwrap();
                pacer.defer(Duration::from_millis(30));
                let start = Instant::now();
                provider::complete(&secret, &messages, &[], |_| {})
                    .await
                    .unwrap();
                assert!(start.elapsed() >= Duration::from_millis(20));
                pacer.defer(Duration::from_millis(30));
                let start = Instant::now();
                provider_attempt::attempt(
                    &secret,
                    &messages,
                    Transport::Stream,
                    Deadlines::default(),
                )
                .await
                .outcome
                .unwrap();
                assert!(start.elapsed() >= Duration::from_millis(20));
                pacer.defer(Duration::from_millis(30));
                let start = Instant::now();
                provider::decide(&secret, &serde_json::json!({}), &serde_json::json!({}))
                    .await
                    .unwrap();
                assert!(start.elapsed() >= Duration::from_millis(20));
                pacer.defer(Duration::from_millis(30));
                let start = Instant::now();
                assert_eq!(
                    provider::transcribe(&secret, b"fixture").await.unwrap(),
                    "transcribed"
                );
                assert!(start.elapsed() >= Duration::from_millis(20));
            })
            .await;
        assert_eq!(server.hits(), 5);
    }

    #[tokio::test]
    async fn queue_time_does_not_spend_the_response_deadline() {
        use crate::provider_attempt::{self, mock, Deadlines, Transport};
        let server = mock::serve(vec![mock::ok_json("done", "stop")]).await;
        let secret = crate::secrets::ProviderSecret {
            kind: "nvidia".into(),
            base_url: server.base_url.clone(),
            model: "nvidia/nemotron-3-super-120b-a12b".into(),
            api_key: Some("fixture-key".into()),
            ..mock::secret(&server.base_url)
        };
        let pacer = Arc::new(Pacer::new(40));
        pacer.defer(Duration::from_millis(200));
        TEST_PACER
            .scope(pacer, async {
                let messages = [crate::provider::ChatMessage {
                    role: "user".into(),
                    content: "hello".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                }];
                let report = provider_attempt::attempt(
                    &secret,
                    &messages,
                    Transport::NonStream,
                    Deadlines {
                        total: Duration::from_millis(100),
                        ..Deadlines::ordinary()
                    },
                )
                .await;
                assert!(report.outcome.is_ok(), "{:?}", report.outcome);
                assert!(report.elapsed_ms >= 190);
            })
            .await;
        assert_eq!(server.hits(), 1);
    }
}
