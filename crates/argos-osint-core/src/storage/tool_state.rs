//! Retained OSINT tool preferences and cached results.

use anyhow::{ensure, Result};
use chrono::Utc;

use super::jobs::rid;
use super::repo::Repository;
use crate::osint::ToolResult;

/// `Store::tool_enabled`: the stored preference, else the tool's default.
pub async fn tool_enabled(repo: &Repository, tool_id: &str) -> Result<bool> {
    let mut r = repo
        .db()
        .query("SELECT VALUE enabled FROM ONLY $p")
        .bind(("p", rid("osint_preference", tool_id)))
        .await?
        .check()?;
    let v: Option<bool> = r.take(0usize)?;
    Ok(v.unwrap_or_else(|| crate::osint::default_enabled(tool_id)))
}

/// `Store::set_tool_enabled` (unknown tools are an error).
pub async fn set_tool_enabled(repo: &Repository, tool_id: &str, enabled: bool) -> Result<()> {
    ensure!(crate::osint::definition(tool_id).is_some(), "unknown tool");
    repo.db()
        .query("UPSERT $p SET enabled = $on RETURN NONE")
        .bind(("p", rid("osint_preference", tool_id)))
        .bind(("on", enabled))
        .await?
        .check()?;
    Ok(())
}

/// `Store::cache_put`: replace the entry, expiring `ttl` seconds from now.
pub async fn cache_put(repo: &Repository, key: &str, result: &ToolResult, ttl: u64) -> Result<()> {
    let expires = (Utc::now() + chrono::Duration::seconds(ttl as i64)).to_rfc3339();
    repo.db()
        .query("UPSERT $k SET result_json = $res, expires_at = $exp RETURN NONE")
        .bind(("k", rid("osint_cache", key)))
        .bind(("res", serde_json::to_string(result)?))
        .bind(("exp", expires))
        .await?
        .check()?;
    Ok(())
}

/// `Store::cache_get_fresh`: an unexpired entry, and (with a cutoff) one
/// retrieved no earlier than the cutoff. `None` cutoff is `cache_get`.
pub async fn cache_get_fresh(
    repo: &Repository,
    key: &str,
    freshness_cutoff: Option<&str>,
) -> Result<Option<ToolResult>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE result_json FROM ONLY $k WHERE expires_at > $now")
        .bind(("k", rid("osint_cache", key)))
        .bind(("now", Utc::now().to_rfc3339()))
        .await?
        .check()?;
    let raw: Option<String> = r.take(0usize)?;
    let Some(res): Option<ToolResult> = raw.and_then(|r| serde_json::from_str(&r).ok()) else {
        return Ok(None);
    };
    if let Some(cutoff) = freshness_cutoff {
        if !cutoff.is_empty() && !res.retrieved_at.is_empty() && res.retrieved_at.as_str() < cutoff
        {
            return Ok(None);
        }
    }
    Ok(Some(res))
}
