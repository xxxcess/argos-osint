//! Public posts. Bluesky, Hacker News, and Mastodon need no key.
//! YouTube search runs only when an API key is set.

use futures_util::future::join_all;
use serde_json::Value;

use super::{
    clip, get_json, get_json_headers, merge_adapter_results, strip_tags, tag, Job, MergeOutcome, SearchHit,
};

const MASTODON: &str = "https://mastodon.social";

pub async fn gather(query: String, youtube_key: Option<String>) -> Result<MergeOutcome, String> {
    let mut jobs: Vec<Job> = Vec::new();
    let bsky = query.clone();
    jobs.push(Box::pin(async move { ("bluesky", bluesky(&bsky).await) }));
    let hn = query.clone();
    jobs.push(Box::pin(async move { ("hn", hacker_news(&hn).await) }));
    let mastodon_query = query.clone();
    jobs.push(Box::pin(async move {
        ("mastodon", mastodon(&mastodon_query).await)
    }));
    if let Some(key) = youtube_key.filter(|key| !key.trim().is_empty()) {
        jobs.push(Box::pin(
            async move { ("youtube", youtube(&query, &key).await) },
        ));
    }
    merge_adapter_results(join_all(jobs).await)
}

async fn bluesky(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q={}&limit=5",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_bluesky(&value), "bluesky"))
}

pub(crate) fn parse_bluesky(value: &Value) -> Vec<SearchHit> {
    let Some(posts) = value.get("posts").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    posts
        .iter()
        .filter_map(|post| {
            let handle = post
                .pointer("/author/handle")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let uri = post.get("uri").and_then(|v| v.as_str()).unwrap_or("");
            let rkey = uri.rsplit('/').next().unwrap_or("");
            if handle.is_empty() || rkey.is_empty() {
                return None;
            }
            let name = post
                .pointer("/author/displayName")
                .and_then(|v| v.as_str())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(handle);
            let text = post
                .pointer("/record/text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(SearchHit {
                title: name.trim().to_string(),
                url: format!("https://bsky.app/profile/{handle}/post/{rkey}"),
                snippet: clip(text, 360),
            })
        })
        .take(5)
        .collect()
}

async fn hacker_news(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://hn.algolia.com/api/v1/search?query={}&tags=story&hitsPerPage=5",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_hn(&value), "hn"))
}

pub(crate) fn parse_hn(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.get("hits").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let object_id = item.get("objectID").and_then(|v| v.as_str()).unwrap_or("");
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if title.is_empty() && object_id.is_empty() {
                return None;
            }
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|url| url.starts_with("http"))
                .map(|url| url.to_string())
                .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={object_id}"));
            let author = item.get("author").and_then(|v| v.as_str()).unwrap_or("");
            let points = item.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
            Some(SearchHit {
                title: if title.is_empty() {
                    format!("HN {object_id}")
                } else {
                    title.to_string()
                },
                url,
                snippet: clip(&format!("{author} · {points} points"), 360),
            })
        })
        .take(5)
        .collect()
}

async fn mastodon(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "{MASTODON}/api/v2/search?q={}&limit=5&resolve=false",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_mastodon(&value), "mastodon"))
}

pub(crate) fn parse_mastodon(value: &Value) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    if let Some(accounts) = value.get("accounts").and_then(|v| v.as_array()) {
        for account in accounts {
            let url = http_url(account.get("url").and_then(|v| v.as_str()).unwrap_or(""));
            let Some(url) = url else { continue };
            let acct = account.get("acct").and_then(|v| v.as_str()).unwrap_or("");
            let name = account
                .get("display_name")
                .and_then(|v| v.as_str())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(acct);
            let note = strip_tags(account.get("note").and_then(|v| v.as_str()).unwrap_or(""));
            hits.push(SearchHit {
                title: name.to_string(),
                url,
                snippet: clip(&note, 360),
            });
        }
    }
    if let Some(statuses) = value.get("statuses").and_then(|v| v.as_array()) {
        for status in statuses {
            let url = http_url(status.get("url").and_then(|v| v.as_str()).unwrap_or(""));
            let Some(url) = url else { continue };
            let content = strip_tags(status.get("content").and_then(|v| v.as_str()).unwrap_or(""));
            let acct = status
                .pointer("/account/acct")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let title = content
                .split_whitespace()
                .take(8)
                .collect::<Vec<_>>()
                .join(" ");
            hits.push(SearchHit {
                title: if title.is_empty() {
                    format!("@{acct}")
                } else {
                    title
                },
                url,
                snippet: clip(&format!("@{acct} {content}"), 360),
            });
        }
    }
    hits.truncate(5);
    hits
}

async fn youtube(query: &str, key: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://www.googleapis.com/youtube/v3/search?part=snippet&type=video&maxResults=5&q={}&key={}",
        urlencoding::encode(query),
        urlencoding::encode(key)
    );
    let value = get_json_headers(&url, &[])
        .await
        .map_err(|err| super::redact(&super::redact(&err, key), &urlencoding::encode(key)))?;
    Ok(tag(parse_youtube(&value), "youtube"))
}

pub(crate) fn parse_youtube(value: &Value) -> Vec<SearchHit> {
    let Some(items) = value.get("items").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item
                .pointer("/id/videoId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id.is_empty() {
                return None;
            }
            let title = item
                .pointer("/snippet/title")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .trim();
            let description = item
                .pointer("/snippet/description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let channel = item
                .pointer("/snippet/channelTitle")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let snippet = if channel.is_empty() {
                description.to_string()
            } else {
                format!("{channel} · {description}")
            };
            Some(SearchHit {
                title: title.to_string(),
                url: format!("https://www.youtube.com/watch?v={id}"),
                snippet: clip(&snippet, 360),
            })
        })
        .take(5)
        .collect()
}

fn http_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with("https://") || value.starts_with("http://") {
        Some(value.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_social_payloads() {
        let bsky: Value = serde_json::from_str(
            r#"{"posts":[{"uri":"at://did:plc:abc/app.bsky.feed.post/xyz","author":{"handle":"ada.bsky.social","displayName":"Ada"},"record":{"text":"hello harbor"}}]}"#,
        )
        .unwrap();
        let hit = &parse_bluesky(&bsky)[0];
        assert_eq!(hit.title, "Ada");
        assert_eq!(hit.url, "https://bsky.app/profile/ada.bsky.social/post/xyz");
        assert_eq!(hit.snippet, "hello harbor");

        let hn: Value = serde_json::from_str(
            r#"{"hits":[{"title":"Show HN","url":null,"author":"ada","points":10,"objectID":"42"}]}"#,
        )
        .unwrap();
        let hit = &parse_hn(&hn)[0];
        assert_eq!(hit.url, "https://news.ycombinator.com/item?id=42");
        assert!(hit.snippet.contains("10"));

        let masto: Value = serde_json::from_str(
            r#"{"accounts":[{"acct":"ada","display_name":"Ada","url":"https://mastodon.social/@ada","note":"<p>hi</p>"}],"statuses":[{"url":"https://mastodon.social/@ada/1","content":"<p>post body</p>","account":{"acct":"ada"}}]}"#,
        )
        .unwrap();
        let hits = parse_mastodon(&masto);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].snippet, "hi");
        assert!(hits[1].snippet.contains("post body"));

        let yt: Value = serde_json::from_str(
            r#"{"items":[{"id":{"videoId":"dQw4w9WgXcQ"},"snippet":{"title":"A talk","description":"desc","channelTitle":"Channel"}}]}"#,
        )
        .unwrap();
        let hit = &parse_youtube(&yt)[0];
        assert_eq!(hit.url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert!(hit.snippet.contains("Channel"));
    }
}
