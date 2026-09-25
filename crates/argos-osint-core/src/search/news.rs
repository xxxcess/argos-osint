//! News: SearXNG's news category when a instance is configured, plus GDELT DOC.

use futures_util::future::join_all;
use serde_json::Value;

use super::web::searx;
use super::{clip, get_json, merge_adapter_results, tag, Job, MergeOutcome, SearchHit};

pub async fn gather(query: String, searx_url: Option<String>) -> Result<MergeOutcome, String> {
    let mut jobs: Vec<Job> = Vec::new();
    if let Some(base) = searx_url.filter(|url| !url.trim().is_empty()) {
        let news_query = query.clone();
        jobs.push(Box::pin(async move {
            (
                "news",
                searx(&base, &news_query, Some("news"))
                    .await
                    .map(|hits| tag(hits, "news")),
            )
        }));
    }
    jobs.push(Box::pin(async move { ("gdelt", gdelt(&query).await) }));
    merge_adapter_results(join_all(jobs).await)
}

async fn gdelt(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://api.gdeltproject.org/api/v2/doc/doc?query={}&mode=ArtList&format=json&maxrecords=8",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_gdelt(&value), "gdelt"))
}

pub(crate) fn parse_gdelt(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.get("articles").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if !url.starts_with("http") {
                return None;
            }
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(url);
            let domain = item.get("domain").and_then(|v| v.as_str()).unwrap_or("");
            let seen = item.get("seendate").and_then(|v| v.as_str()).unwrap_or("");
            let country = item
                .get("sourcecountry")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let snippet = [domain, seen, country]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            Some(SearchHit {
                title: title.trim().to_string(),
                url: url.to_string(),
                snippet: clip(&snippet, 360),
            })
        })
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gdelt_articles() {
        let value: Value = serde_json::from_str(
            r#"{"articles":[{"title":"Harbor strike","url":"https://news.example/a","seendate":"20260924T120000Z","domain":"news.example","sourcecountry":"US"}]}"#,
        )
        .unwrap();
        let hits = parse_gdelt(&value);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Harbor strike");
        assert!(hits[0].snippet.contains("news.example"));
        assert!(hits[0].snippet.contains("US"));
    }
}
