//! Automatic full-article retrieval with Firecrawl-first fallbacks (no SociaVault).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::osint::{self, ProviderKeys};
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, Store};

use super::replace_insights::replace_article_insights_from_body;
use super::synthesize::refine_retrieved_article_body;
use super::validate::{validate_article_body, BodyQuality};

/// Default cooldown after a terminal retrieval failure.
pub const FAILURE_COOLDOWN_SECS: i64 = 15 * 60;

#[derive(Clone, Debug)]
pub enum BodyFetchEvent {
    Started {
        article_id: String,
        body_id: String,
        generation: i64,
    },
    Attempt {
        article_id: String,
        body_id: String,
        tool_id: String,
        state: String,
        reason: String,
        generation: i64,
    },
    Progress {
        article_id: String,
        body_id: String,
        message: String,
        generation: i64,
    },
    /// Cleaned body is committed and ready to display; insight replace may still be running.
    Ready {
        article_id: String,
        body_id: String,
        quality: String,
        generation: i64,
    },
    /// Body is on screen; Brain insight re-extract from the cleaned body has started.
    InsightsRefreshing {
        article_id: String,
        body_id: String,
        generation: i64,
    },
    /// Brain insights for the article were replaced from the cleaned body.
    InsightsReplaced {
        article_id: String,
        body_id: String,
        claim_count: usize,
        generation: i64,
    },
    /// Insight re-extract finished unsuccessfully; clear insight loading overlays.
    InsightsFailed {
        article_id: String,
        body_id: String,
        reason: String,
        generation: i64,
    },
    Failed {
        article_id: String,
        body_id: String,
        reason: String,
        generation: i64,
    },
}

/// Ordered capability-eligible routes for automatic article-body acquisition.
/// SociaVault tools are never included.
pub fn body_fetch_routes() -> Vec<&'static str> {
    vec![
        "firecrawl_scrape",
        "firecrawl_search", // discovery only — locate canonical/print/AMP, then scrape
        "direct_http_extract",
        "wayback_availability", // discovery — locate archive snapshot URL
    ]
}

fn is_sociavault(tool_id: &str) -> bool {
    let id = tool_id.to_ascii_lowercase();
    id.contains("sociavault") || id == "google_search"
}

fn has_firecrawl_key(keys: &ProviderKeys) -> bool {
    let (primary, fallback) = keys.pair("firecrawl");
    !primary.trim().is_empty() || !fallback.trim().is_empty()
}

/// Enqueue or subscribe: returns existing ready body, respects cooldown, or starts fetch.
pub fn enqueue_article_body(
    store: &Store,
    article: &AtlasArticleRow,
    force_refresh: bool,
) -> Result<EnqueueOutcome> {
    let body = store.ensure_article_body(
        &article.id,
        &article.run_id,
        &article.url,
        &article.source_domain,
        &article.source_name,
    )?;

    let usable = matches!(body.quality.as_str(), "complete" | "partial" | "uncertain")
        && !body.body_markdown.trim().is_empty();

    if usable && !force_refresh {
        return Ok(EnqueueOutcome::Cached(body.id));
    }

    if body.state == "running" {
        return Ok(EnqueueOutcome::AlreadyRunning {
            body_id: body.id,
            generation: body.body_version,
        });
    }

    if !force_refresh && !body.retry_after.is_empty() {
        if let Ok(until) = chrono::DateTime::parse_from_rfc3339(&body.retry_after) {
            if until > chrono::Utc::now() {
                return Ok(EnqueueOutcome::Cooldown {
                    body_id: body.id,
                    retry_after: body.retry_after,
                    reason: body.quality_rationale,
                });
            }
        }
    }

    if force_refresh {
        store.clear_article_body_content(&body.id)?;
    } else {
        store.update_article_body_state(&body.id, "running")?;
    }
    Ok(EnqueueOutcome::Start {
        body_id: body.id,
        generation: if force_refresh {
            body.body_version + 1
        } else {
            body.body_version
        },
        force_refresh,
    })
}

#[derive(Clone, Debug)]
pub enum EnqueueOutcome {
    Cached(String),
    AlreadyRunning { body_id: String, generation: i64 },
    Cooldown {
        body_id: String,
        retry_after: String,
        reason: String,
    },
    Start {
        body_id: String,
        generation: i64,
        force_refresh: bool,
    },
}

/// Run the full fallback chain for one article body. Safe to call from a worker task.
/// Opens a fresh Store around each await so the future stays `Send`.
///
/// When `synthesis` is set, a usable scrape is passed through the synthesis model so
/// only brief-relevant article prose is committed. When `classifier` is set, a second
/// pass removes sponsored / unrelated-link spans by absolute character ranges.
pub async fn fetch_article_body(
    db_path: &std::path::Path,
    body_id: &str,
    title: &str,
    brief: &str,
    keys: ProviderKeys,
    synthesis: Option<ProviderSecret>,
    classifier: Option<ProviderSecret>,
    user_agent: Option<String>,
    force_refresh: bool,
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(BodyFetchEvent) + Send,
) -> Result<()> {
    let (generation, article_id, url, source_domain) = {
        let store = Store::open(db_path)?;
        let body = store
            .article_body_by_id(body_id)?
            .ok_or_else(|| anyhow!("article body {body_id} missing"))?;
        (
            body.body_version,
            body.article_id,
            body.original_url,
            body.source_domain,
        )
    };
    let brief = brief.to_string();

    on_event(BodyFetchEvent::Started {
        article_id: article_id.clone(),
        body_id: body_id.into(),
        generation,
    });

    let executor = osint::Executor::new()?;
    let mut best_partial = String::new();
    let mut best_tool = String::new();
    let mut best_resolved = url.clone();
    let mut attempted = Vec::new();

    for route in body_fetch_routes() {
        if cancel.load(Ordering::Relaxed) {
            let store = Store::open(db_path)?;
            store.update_article_body_state(body_id, "idle")?;
            return Ok(());
        }
        if is_sociavault(route) {
            let store = Store::open(db_path)?;
            let _ = store.insert_retrieval_attempt(
                body_id,
                route,
                &url,
                "skipped",
                "SociaVault excluded from automatic article-body acquisition",
                "",
                generation,
            )?;
            continue;
        }

        on_event(BodyFetchEvent::Attempt {
            article_id: article_id.clone(),
            body_id: body_id.into(),
            tool_id: route.into(),
            state: "running".into(),
            reason: String::new(),
            generation,
        });
        on_event(BodyFetchEvent::Progress {
            article_id: article_id.clone(),
            body_id: body_id.into(),
            message: format!("Trying {route}"),
            generation,
        });

        let attempt_id = {
            let store = Store::open(db_path)?;
            store.insert_retrieval_attempt(
                body_id,
                route,
                &url,
                "running",
                "",
                "",
                generation,
            )?
        };
        attempted.push(route.to_string());

        let outcome = match route {
            "firecrawl_scrape" => {
                scrape_via_firecrawl(&executor, &keys, user_agent.as_deref(), &url).await
            }
            "firecrawl_search" => {
                discover_and_scrape(
                    &executor,
                    &keys,
                    user_agent.as_deref(),
                    title,
                    &url,
                    &source_domain,
                )
                .await
            }
            "direct_http_extract" => direct_http_extract(&url, user_agent.as_deref()).await,
            "wayback_availability" => {
                wayback_then_fetch(&executor, &keys, user_agent.as_deref(), &url).await
            }
            other => Err(anyhow!("route {other} not applicable")),
        };

        let store = Store::open(db_path)?;
        match outcome {
            Ok(fetched) => {
                let validation = validate_article_body(&fetched.markdown, title, &url);
                let reason = validation.rationale.clone();
                store.finish_retrieval_attempt(
                    &attempt_id,
                    match validation.quality {
                        BodyQuality::Complete | BodyQuality::Partial | BodyQuality::Uncertain => {
                            "succeeded"
                        }
                        BodyQuality::Unavailable => "failed",
                    },
                    &reason,
                    "",
                )?;
                on_event(BodyFetchEvent::Attempt {
                    article_id: article_id.clone(),
                    body_id: body_id.into(),
                    tool_id: route.into(),
                    state: "finished".into(),
                    reason: reason.clone(),
                    generation,
                });

                match validation.quality {
                    BodyQuality::Complete => {
                        drop(store);
                        commit_refined_body(
                            db_path,
                            body_id,
                            title,
                            &brief,
                            &url,
                            &validation.cleaned_markdown,
                            &fetched.resolved_url,
                            route,
                            force_refresh,
                            generation,
                            &article_id,
                            synthesis.as_ref(),
                            classifier.as_ref(),
                            &mut on_event,
                        )
                        .await?;
                        return Ok(());
                    }
                    BodyQuality::Partial | BodyQuality::Uncertain => {
                        if validation.cleaned_markdown.len() > best_partial.len() {
                            best_partial = validation.cleaned_markdown;
                            best_tool = route.into();
                            best_resolved = fetched.resolved_url;
                        }
                    }
                    BodyQuality::Unavailable => {}
                }
            }
            Err(err) => {
                let reason = err.to_string();
                store.finish_retrieval_attempt(&attempt_id, "failed", &reason, "")?;
                on_event(BodyFetchEvent::Attempt {
                    article_id: article_id.clone(),
                    body_id: body_id.into(),
                    tool_id: route.into(),
                    state: "failed".into(),
                    reason: reason.clone(),
                    generation,
                });
            }
        }
    }

    if !best_partial.is_empty() {
        commit_refined_body(
            db_path,
            body_id,
            title,
            &brief,
            &url,
            &best_partial,
            &best_resolved,
            &best_tool,
            force_refresh,
            generation,
            &article_id,
            synthesis.as_ref(),
            classifier.as_ref(),
            &mut on_event,
        )
        .await?;
        return Ok(());
    }

    let store = Store::open(db_path)?;

    let retry_after =
        (chrono::Utc::now() + chrono::Duration::seconds(FAILURE_COOLDOWN_SECS)).to_rfc3339();
    let rationale = format!(
        "All eligible body routes exhausted ({})",
        attempted.join(", ")
    );
    store.mark_article_body_failed(body_id, &rationale, &retry_after, "", "unavailable")?;
    on_event(BodyFetchEvent::Failed {
        article_id,
        body_id: body_id.into(),
        reason: rationale,
        generation,
    });
    Ok(())
}

async fn commit_refined_body(
    db_path: &std::path::Path,
    body_id: &str,
    title: &str,
    brief: &str,
    url: &str,
    scraped_markdown: &str,
    resolved_url: &str,
    tool: &str,
    force_refresh: bool,
    generation: i64,
    article_id: &str,
    synthesis: Option<&ProviderSecret>,
    classifier: Option<&ProviderSecret>,
    on_event: &mut (impl FnMut(BodyFetchEvent) + Send),
) -> Result<()> {
    if synthesis.is_some() {
        on_event(BodyFetchEvent::Progress {
            article_id: article_id.into(),
            body_id: body_id.into(),
            message: "Extracting brief-relevant article body…".into(),
            generation,
        });
    } else {
        on_event(BodyFetchEvent::Progress {
            article_id: article_id.into(),
            body_id: body_id.into(),
            message: "Saving retrieved article body…".into(),
            generation,
        });
    }
    if classifier.is_some() {
        on_event(BodyFetchEvent::Progress {
            article_id: article_id.into(),
            body_id: body_id.into(),
            message: "Classifying sponsored and unrelated sections…".into(),
            generation,
        });
    }
    let refined = refine_retrieved_article_body(
        synthesis,
        classifier,
        title,
        brief,
        url,
        scraped_markdown,
    )
    .await;
    let store = Store::open(db_path)?;
    store.commit_article_body(
        body_id,
        &refined.markdown,
        &refined.content_hash,
        refined.quality.as_str(),
        &refined.rationale,
        resolved_url,
        tool,
        force_refresh,
    )?;
    // Emit Ready first so the cleaned body can paint while insights re-extract.
    on_event(BodyFetchEvent::Ready {
        article_id: article_id.into(),
        body_id: body_id.into(),
        quality: refined.quality.as_str().into(),
        generation,
    });
    let usable_body = matches!(refined.quality.as_str(), "complete" | "partial" | "uncertain")
        && !refined.markdown.trim().is_empty();
    if usable_body {
        if let Some(synthesis) = synthesis {
            let article = {
                let body_row = store
                    .article_body_by_id(body_id)?
                    .ok_or_else(|| anyhow!("article body {body_id} missing after commit"))?;
                store.atlas_article(&body_row.run_id, article_id)?
            };
            drop(store);
            if let Some(article) = article {
                on_event(BodyFetchEvent::InsightsRefreshing {
                    article_id: article_id.into(),
                    body_id: body_id.into(),
                    generation,
                });
                on_event(BodyFetchEvent::Progress {
                    article_id: article_id.into(),
                    body_id: body_id.into(),
                    message: "Re-extracting insights from full article…".into(),
                    generation,
                });
                match replace_article_insights_from_body(
                    db_path,
                    &article,
                    &refined.markdown,
                    synthesis,
                    classifier,
                )
                .await
                {
                    Ok(claim_count) => {
                        on_event(BodyFetchEvent::InsightsReplaced {
                            article_id: article_id.into(),
                            body_id: body_id.into(),
                            claim_count,
                            generation,
                        });
                    }
                    Err(err) => {
                        on_event(BodyFetchEvent::InsightsFailed {
                            article_id: article_id.into(),
                            body_id: body_id.into(),
                            reason: err.to_string(),
                            generation,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

struct FetchedBody {
    markdown: String,
    resolved_url: String,
}

async fn scrape_via_firecrawl(
    executor: &osint::Executor,
    keys: &ProviderKeys,
    user_agent: Option<&str>,
    url: &str,
) -> Result<FetchedBody> {
    if !has_firecrawl_key(keys) {
        return Err(anyhow!("Firecrawl credentials missing"));
    }
    let result = executor
        .run_configured(
            "firecrawl_scrape",
            json!({"url": url, "formats": ["markdown"]}),
            user_agent,
            keys,
        )
        .await?;
    if let Some(err) = result.error {
        return Err(anyhow!(err));
    }
    let markdown = full_markdown_from_result(&result)?;
    let resolved = result
        .observations
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or(url)
        .to_string();
    Ok(FetchedBody {
        markdown,
        resolved_url: resolved,
    })
}

/// Prefer unclipped markdown from the raw Firecrawl payload.
fn full_markdown_from_result(result: &osint::ToolResult) -> Result<String> {
    if let Ok(v) = serde_json::from_str::<Value>(&result.raw) {
        let data = v.get("data").unwrap_or(&v);
        if let Some(md) = data
            .get("markdown")
            .or_else(|| data.get("content"))
            .and_then(Value::as_str)
        {
            if !md.trim().is_empty() {
                return Ok(md.to_string());
            }
        }
    }
    let md = result
        .observations
        .get("markdown")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if md.trim().is_empty() {
        return Err(anyhow!("no markdown in Firecrawl response"));
    }
    Ok(md)
}

async fn discover_and_scrape(
    executor: &osint::Executor,
    keys: &ProviderKeys,
    user_agent: Option<&str>,
    title: &str,
    url: &str,
    domain: &str,
) -> Result<FetchedBody> {
    if !has_firecrawl_key(keys) {
        return Err(anyhow!("Firecrawl credentials missing"));
    }
    let query = if title.trim().is_empty() {
        format!("site:{domain}")
    } else {
        format!("{} site:{}", title.chars().take(80).collect::<String>(), domain)
    };
    let result = executor
        .run_configured(
            "firecrawl_search",
            json!({"query": query, "limit": 5}),
            user_agent,
            keys,
        )
        .await?;
    if let Some(err) = result.error {
        return Err(anyhow!(err));
    }
    let candidates = search_urls(&result.observations);
    let mut tried = 0;
    for candidate in candidates {
        if !same_registrable_hint(&candidate, domain) && candidate != url {
            continue;
        }
        if !looks_like_article_url(&candidate) && candidate != url {
            continue;
        }
        tried += 1;
        if let Ok(fetched) = scrape_via_firecrawl(executor, keys, user_agent, &candidate).await {
            return Ok(fetched);
        }
        if tried >= 3 {
            break;
        }
    }
    Err(anyhow!("Firecrawl search found no fetchable canonical article URL"))
}

fn search_urls(obs: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(items) = obs.as_array() {
        for item in items {
            if let Some(u) = item.get("url").and_then(Value::as_str) {
                out.push(u.to_string());
            }
        }
    } else if let Some(items) = obs.get("results").and_then(Value::as_array) {
        for item in items {
            if let Some(u) = item.get("url").and_then(Value::as_str) {
                out.push(u.to_string());
            }
        }
    }
    out
}

fn same_registrable_hint(url: &str, domain: &str) -> bool {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return true;
    }
    url.to_ascii_lowercase().contains(&domain)
}

fn looks_like_article_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("/amp")
        || lower.contains("print")
        || lower.contains("/article")
        || lower.contains("/news/")
        || lower.contains("/story")
        || lower.chars().filter(|c| *c == '/').count() >= 4
}

async fn wayback_then_fetch(
    executor: &osint::Executor,
    keys: &ProviderKeys,
    user_agent: Option<&str>,
    url: &str,
) -> Result<FetchedBody> {
    let result = executor
        .run_configured("wayback_availability", json!({"url": url}), user_agent, keys)
        .await?;
    if let Some(err) = result.error {
        return Err(anyhow!(err));
    }
    let snapshot = result
        .observations
        .pointer("/url")
        .or_else(|| result.observations.pointer("/snapshot"))
        .or_else(|| result.observations.get("closest").and_then(|c| c.get("url")))
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let Some(snapshot) = snapshot else {
        return Err(anyhow!("no Wayback snapshot URL"));
    };
    // Prefer Firecrawl for the snapshot; fall back to direct HTTP.
    if has_firecrawl_key(keys) {
        if let Ok(fetched) = scrape_via_firecrawl(executor, keys, user_agent, &snapshot).await {
            return Ok(fetched);
        }
    }
    direct_http_extract(&snapshot, user_agent).await
}

async fn direct_http_extract(url: &str, user_agent: Option<&str>) -> Result<FetchedBody> {
    let parsed = url::Url::parse(url)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("url host missing"))?;
    // Private-network protection.
    if is_private_host(host) {
        return Err(anyhow!("private-network URL blocked"));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(user_agent.unwrap_or(
            "ArgosOSINT/1.0 (+https://github.com/argos; article-body; contact@localhost)",
        ))
        .build()?;
    let response = client
        .get(parsed.clone())
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await?
        .error_for_status()?;
    let final_url = response.url().to_string();
    let bytes = response.bytes().await?;
    if bytes.len() > 2_000_000 {
        return Err(anyhow!("response too large"));
    }
    let html = String::from_utf8_lossy(&bytes);
    let markdown = html_main_content(&html);
    if markdown.trim().is_empty() {
        return Err(anyhow!("direct extract produced no main content"));
    }
    Ok(FetchedBody {
        markdown,
        resolved_url: final_url,
    })
}

fn is_private_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".local")
        || h.starts_with("127.")
        || h.starts_with("10.")
        || h.starts_with("192.168.")
        || h.starts_with("169.254.")
        || h == "::1"
        || h.starts_with("fc")
        || h.starts_with("fd")
}

/// Lightweight main-content extractor for HTML pages.
fn html_main_content(html: &str) -> String {
    let mut text = String::new();
    let lower = html.to_ascii_lowercase();
    let start = lower
        .find("<article")
        .or_else(|| lower.find("<main"))
        .or_else(|| lower.find("<body"))
        .unwrap_or(0);
    let slice = &html[start..];
    let mut in_tag = false;
    let mut in_script = false;
    let mut tag = String::new();
    for ch in slice.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag
                    .trim()
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if name == "script" || name == "style" || name == "noscript" {
                    in_script = !tag.trim().starts_with('/');
                }
                if matches!(name.as_str(), "p" | "br" | "h1" | "h2" | "h3" | "li" | "div")
                    && !tag.trim().starts_with('/')
                {
                    text.push('\n');
                }
                tag.clear();
            }
            c if in_tag => tag.push(c),
            c if !in_script => text.push(c),
            _ => {}
        }
    }
    // Decode a few common entities.
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    let mut lines = Vec::new();
    for line in text.lines() {
        let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            continue;
        }
        lines.push(t);
    }
    lines.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_exclude_sociavault() {
        for route in body_fetch_routes() {
            assert!(!is_sociavault(route), "{route}");
        }
    }

    #[test]
    fn private_hosts_blocked() {
        assert!(is_private_host("127.0.0.1"));
        assert!(is_private_host("localhost"));
        assert!(!is_private_host("example.com"));
    }

    #[test]
    fn html_extract_keeps_paragraphs() {
        let html = r#"<html><body><nav>Menu</nav><article><h1>Title</h1><p>First paragraph with enough words for a test.</p><p>Second paragraph continues the story.</p></article></body></html>"#;
        let md = html_main_content(html);
        assert!(md.contains("First paragraph"));
        assert!(md.contains("Second paragraph"));
    }
}
