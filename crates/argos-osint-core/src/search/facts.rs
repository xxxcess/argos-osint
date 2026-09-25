//! Wikipedia full-text search and Wikidata entities.

use futures_util::future::join_all;
use serde_json::Value;

use super::{clip, get_json, merge_adapter_results, tag, Job, SearchHit};

pub async fn gather(terms: Vec<String>) -> Result<Vec<SearchHit>, String> {
    let mut jobs: Vec<Job> = Vec::new();
    for term in terms.into_iter().take(2) {
        let wiki = term.clone();
        jobs.push(Box::pin(
            async move { ("wikipedia", wikipedia(&wiki).await) },
        ));
        jobs.push(Box::pin(async move { ("wikidata", wikidata(&term).await) }));
    }
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    merge_adapter_results(join_all(jobs).await)
}

async fn wikipedia(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srlimit=5&utf8=1&format=json&srsearch={}",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    Ok(tag(parse_wikipedia_search(&value), "wikipedia"))
}

async fn wikidata(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://www.wikidata.org/w/api.php?action=wbsearchentities&search={}&language=en&format=json&limit=5&type=item",
        urlencoding::encode(query)
    );
    let value = get_json(&url).await?;
    let search_hits = tag(parse_wikidata_search(&value), "wikidata");
    let ids = wikidata_ids(&value);
    if ids.is_empty() {
        return Ok(search_hits);
    }
    let entity_url = format!(
        "https://www.wikidata.org/w/api.php?action=wbgetentities&ids={}&props=labels|descriptions|sitelinks&languages=en&format=json",
        urlencoding::encode(&ids.join("|"))
    );
    match get_json(&entity_url).await {
        Ok(entities) => {
            let rich = tag(parse_wikidata_entities(&entities), "wikidata");
            if rich.is_empty() {
                Ok(search_hits)
            } else {
                Ok(rich)
            }
        }
        Err(_) => Ok(search_hits),
    }
}

pub fn parse_wikipedia_opensearch(value: &Value) -> Vec<SearchHit> {
    let titles = value.get(1).and_then(|v| v.as_array());
    let snippets = value.get(2).and_then(|v| v.as_array());
    let urls = value.get(3).and_then(|v| v.as_array());
    let Some(titles) = titles else {
        return Vec::new();
    };
    titles
        .iter()
        .enumerate()
        .filter_map(|(i, title)| {
            let title = title.as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            let url = urls
                .and_then(|rows| rows.get(i))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                return None;
            }
            let snippet = snippets
                .and_then(|rows| rows.get(i))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(SearchHit {
                title: title.to_string(),
                url,
                snippet,
            })
        })
        .take(5)
        .collect()
}

/// Full-text Wikipedia search (`list=search`). OpenSearch only matches titles,
/// so a question such as "who is elon musk" comes back empty.
pub fn parse_wikipedia_search(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value
        .get("query")
        .and_then(|query| query.get("search"))
        .and_then(|search| search.as_array())
    else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let title = item.get("title").and_then(|v| v.as_str())?.trim();
            if title.is_empty() {
                return None;
            }
            let snippet =
                super::strip_tags(item.get("snippet").and_then(|v| v.as_str()).unwrap_or(""));
            Some(SearchHit {
                url: wikipedia_url(title),
                title: title.to_string(),
                snippet: clip(&snippet, 360),
            })
        })
        .take(5)
        .collect()
}

fn wikipedia_url(title: &str) -> String {
    let slug = title.replace(' ', "_");
    format!(
        "https://en.wikipedia.org/wiki/{}",
        urlencoding::encode(&slug)
    )
}

pub(crate) fn parse_wikidata_search(value: &Value) -> Vec<SearchHit> {
    let Some(rows) = value.get("search").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let label = item
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .trim();
            if label.is_empty() {
                return None;
            }
            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(SearchHit {
                title: label.to_string(),
                url: wikidata_url(id, item.get("concepturi").and_then(|v| v.as_str())),
                snippet: clip(description, 360),
            })
        })
        .take(5)
        .collect()
}

pub(crate) fn wikidata_ids(value: &Value) -> Vec<String> {
    value
        .get("search")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|item| {
                    item.get("id")
                        .and_then(|v| v.as_str())
                        .map(|id| id.to_string())
                })
                .take(3)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn parse_wikidata_entities(value: &Value) -> Vec<SearchHit> {
    let Some(entities) = value.get("entities").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for (id, entity) in entities {
        if entity.get("missing").is_some() {
            continue;
        }
        let label = entity
            .pointer("/labels/en/value")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .trim();
        if label.is_empty() {
            continue;
        }
        let description = entity
            .pointer("/descriptions/en/value")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let sitelink = entity.pointer("/sitelinks/enwiki");
        let url = sitelink
            .and_then(|link| link.get("url").and_then(|v| v.as_str()))
            .filter(|url| url.starts_with("http"))
            .map(|url| url.to_string())
            .or_else(|| {
                sitelink
                    .and_then(|link| link.get("title").and_then(|v| v.as_str()))
                    .map(wikipedia_url)
            })
            .unwrap_or_else(|| format!("https://www.wikidata.org/wiki/{id}"));
        let snippet = if description.is_empty() {
            format!("Wikidata {id}")
        } else {
            format!("{description} ({id})")
        };
        hits.push(SearchHit {
            title: label.to_string(),
            url,
            snippet: clip(&snippet, 360),
        });
        if hits.len() == 3 {
            break;
        }
    }
    hits
}

fn wikidata_url(id: &str, concepturi: Option<&str>) -> String {
    if let Some(uri) = concepturi.map(str::trim).filter(|uri| !uri.is_empty()) {
        if let Some(rest) = uri.strip_prefix("//") {
            return format!("https://{rest}");
        }
        if let Some(rest) = uri.strip_prefix("http://") {
            if rest.starts_with("www.wikidata.org") {
                return format!("https://{rest}");
            }
        }
        if uri.starts_with("http://") || uri.starts_with("https://") {
            return uri.to_string();
        }
    }
    if id.is_empty() {
        "https://www.wikidata.org".into()
    } else {
        format!("https://www.wikidata.org/wiki/{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wikipedia_full_text_search() {
        let raw = r#"{"query":{"search":[{"title":"Elon Musk","snippet":"<span class=\"searchmatch\">Elon</span> Musk is a businessman"}]}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let hits = parse_wikipedia_search(&value);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Elon Musk");
        assert_eq!(hits[0].url, "https://en.wikipedia.org/wiki/Elon_Musk");
        assert!(hits[0].snippet.contains("Elon Musk is a businessman"));
    }

    #[test]
    fn parses_wikipedia_opensearch() {
        let raw = r#"["ada",["Ada Lovelace"],["Mathematician"],["https://en.wikipedia.org/wiki/Ada_Lovelace"]]"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let hits = parse_wikipedia_opensearch(&value);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Ada Lovelace");
        assert!(hits[0].url.contains("Ada_Lovelace"));
    }

    #[test]
    fn parses_wikidata_search_and_entities() {
        let search: Value = serde_json::from_str(
            r#"{"search":[{"id":"Q42","label":"Douglas Adams","description":"English writer","concepturi":"http://www.wikidata.org/entity/Q42"}]}"#,
        )
        .unwrap();
        let hits = parse_wikidata_search(&search);
        assert_eq!(hits[0].title, "Douglas Adams");
        assert_eq!(hits[0].url, "https://www.wikidata.org/entity/Q42");
        assert_eq!(wikidata_ids(&search), vec!["Q42".to_string()]);

        let entities: Value = serde_json::from_str(
            r#"{"entities":{"Q42":{"labels":{"en":{"value":"Douglas Adams"}},"descriptions":{"en":{"value":"English writer"}},"sitelinks":{"enwiki":{"title":"Douglas Adams","url":"https://en.wikipedia.org/wiki/Douglas_Adams"}}}}}"#,
        )
        .unwrap();
        let hits = parse_wikidata_entities(&entities);
        assert_eq!(hits[0].url, "https://en.wikipedia.org/wiki/Douglas_Adams");
        assert!(hits[0].snippet.contains("Q42"));
    }
}
