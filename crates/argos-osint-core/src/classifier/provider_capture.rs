//! Exact bounded provider metadata capture before Atlas filtering.
//! Provider categories and AI tags remain separate, untrusted observations.
use crate::osint::atlas_news::{CURRENTS_HOST, NEWSDATA_HOST};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CaptureContext {
    pub cycle_id: String,
    pub fetched_at: String,
    pub fetch_receipt_key: String,
    pub request_input_hash: String,
}

impl CaptureContext {
    /// Identifies the already-fetched response; capture validates the bounds.
    #[must_use]
    pub fn new(
        cycle_id: impl Into<String>,
        fetched_at: impl Into<String>,
        fetch_receipt_key: impl Into<String>,
        request_input_hash: impl Into<String>,
    ) -> Self {
        Self {
            cycle_id: cycle_id.into(),
            fetched_at: fetched_at.into(),
            fetch_receipt_key: fetch_receipt_key.into(),
            request_input_hash: request_input_hash.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CaptureRejection {
    pub article_identity_hash: String,
    pub code: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CaptureBatch {
    pub observations: Vec<crate::classifier::provider_labels::ProviderObservation>,
    pub rejections: Vec<CaptureRejection>,
}
const CAPTURE_FIELDS: &[&str] = &[
    "title",
    "description",
    "url",
    "link",
    "article_id",
    "id",
    "source_name",
    "source_id",
    "source",
    "country",
    "language",
    "pubDate",
    "published",
    "publishedAt",
    "published_at",
    "updated",
    "updated_at",
    "category",
    "categories",
    "ai_tag",
    "ai_tags",
];

/// Exact unfiltered category observations from a fetched response. No extra
/// request is made, and this function never invokes the Hit clipping/filtering.
pub fn capture_categories(
    format: crate::classifier::provider_labels::ProviderFormat,
    endpoint: &str,
    value: &serde_json::Value,
    context: &CaptureContext,
) -> CaptureBatch {
    let count = capture_raw_rows(&format, value).map_or(0, |rows| rows.len());
    let mut result = CaptureBatch::default();
    if count == 0 {
        return capture_categories_page(format, endpoint, value, context, 0);
    }
    for offset in (0..count).step_by(crate::classifier::INFERENCE_BATCH_SIZE) {
        let mut page = capture_categories_page(format.clone(), endpoint, value, context, offset);
        result.observations.append(&mut page.observations);
        result.rejections.append(&mut page.rejections);
    }
    result
}
fn capture_raw_rows<'a>(
    format: &crate::classifier::provider_labels::ProviderFormat,
    value: &'a serde_json::Value,
) -> Option<&'a [serde_json::Value]> {
    use crate::classifier::provider_labels::ProviderFormat;
    match format {
        ProviderFormat::NewsDataV1 => value.get("results"),
        ProviderFormat::CurrentsV1 => value.get("news"),
        ProviderFormat::Unsupported { provider, .. } if provider == "newsdata" => {
            value.get("results")
        }
        ProviderFormat::Unsupported { provider, .. } if provider == "currents" => {
            value.get("news").or_else(|| value.get("data"))
        }
        ProviderFormat::Unsupported { .. } => None,
    }
    .and_then(serde_json::Value::as_array)
    .map(Vec::as_slice)
}
/// Foreground capture processes one borrowed raw slice at a time, keeping at
/// most64 successful128KiB observations alive before its monitoring commit.
pub(crate) fn capture_categories_page(
    format: crate::classifier::provider_labels::ProviderFormat,
    endpoint: &str,
    value: &serde_json::Value,
    context: &CaptureContext,
    offset: usize,
) -> CaptureBatch {
    use crate::classifier::provider_labels::{self, ProviderFormat, ProviderObservation};
    let mut result = CaptureBatch::default();
    let reject = |identity: &str, code: &str, bytes: usize| CaptureRejection {
        article_identity_hash: crate::classifier::artifact::sha256_hex(identity.as_bytes()),
        code: code.into(),
        bytes: bytes as u64,
    };
    if !crate::intel_service::valid_id(&context.cycle_id)
        || context.fetch_receipt_key.is_empty()
        || context.fetch_receipt_key.len() > 4096
        || context.fetch_receipt_key.chars().any(char::is_control)
        || context.request_input_hash.len() != 64
        || !context
            .request_input_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || chrono::DateTime::parse_from_rfc3339(&context.fetched_at).is_err()
    {
        result
            .rejections
            .push(reject("context", "invalid_capture_context", 0));
        return result;
    }
    let Ok(url) = url::Url::parse(endpoint) else {
        result
            .rejections
            .push(reject("endpoint", "invalid_capture_endpoint", 0));
        return result;
    };
    let Some(host) = url.host_str() else {
        result
            .rejections
            .push(reject("endpoint", "invalid_capture_endpoint", 0));
        return result;
    };
    if url.path().len() > 512 || !matches!(url.scheme(), "https" | "http") {
        result
            .rejections
            .push(reject("endpoint", "invalid_capture_endpoint", 0));
        return result;
    }
    let sanitized = format!("{}://{}{}", url.scheme(), host, url.path());
    let actual = match format {
        ProviderFormat::NewsDataV1 if host == NEWSDATA_HOST && url.path() == "/api/1/latest" => {
            ProviderFormat::NewsDataV1
        }
        ProviderFormat::CurrentsV1 if host == CURRENTS_HOST && url.path() == "/v1/latest-news" => {
            ProviderFormat::CurrentsV1
        }
        ProviderFormat::NewsDataV1 => ProviderFormat::Unsupported {
            provider: "newsdata".into(),
            version: url.path().into(),
        },
        ProviderFormat::CurrentsV1 => ProviderFormat::Unsupported {
            provider: "currents".into(),
            version: url.path().into(),
        },
        unsupported => unsupported,
    };
    let failed = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("error"))
        || value
            .get("error")
            .is_some_and(|error| !error.is_null() && error != false);
    if failed {
        if offset == 0 {
            result
                .rejections
                .push(reject(&sanitized, "provider_response_error", 0));
        }
        return result;
    }
    let Some(rows) = capture_raw_rows(&actual, value) else {
        if offset == 0 {
            result
                .rejections
                .push(reject(&sanitized, "provider_response_shape", 0));
        }
        return result;
    };
    let Some(rows) = rows.get(
        offset
            ..rows
                .len()
                .min(offset.saturating_add(crate::classifier::INFERENCE_BATCH_SIZE)),
    ) else {
        return result;
    };
    for row in rows {
        let Some(object) = row.as_object() else {
            result
                .rejections
                .push(reject("article", "article_not_object", 0));
            continue;
        };
        let exact_text = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
                .unwrap_or("")
        };
        let exact_url = exact_text(&["link", "url"]);
        let supplied_id = exact_text(&["article_id", "id"]);
        let identity = if supplied_id.is_empty() {
            exact_url
        } else {
            supplied_id
        };
        if identity.is_empty() {
            result
                .rejections
                .push(reject("article", "missing_article_identity", 0));
            continue;
        }
        // Borrow the allowlisted metadata while checking its serialized bound;
        // content/full_text/request/auth fields never get copied into capture.
        let borrowed: std::collections::BTreeMap<_, _> = CAPTURE_FIELDS
            .iter()
            .filter_map(|key| object.get(*key).map(|value| (*key, value)))
            .filter(|(key, _)| *key != "source")
            .collect();
        let borrowed_source: std::collections::BTreeMap<_, _> = object
            .get("source")
            .and_then(serde_json::Value::as_object)
            .map(|source| {
                ["id", "name", "country", "domain"]
                    .into_iter()
                    .filter_map(|key| {
                        source
                            .get(key)
                            .filter(|value| value.is_string() || value.is_null())
                            .map(|value| (key, value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let source_text = object.get("source").filter(|source| source.is_string());
        if !capture_depth_bounded(borrowed.values().copied()) {
            result
                .rejections
                .push(reject(identity, "capture_metadata_depth", 0));
            continue;
        }
        let Ok(bytes) = capture_json_size(
            &(&borrowed, &borrowed_source, source_text),
            provider_labels::MAX_OBSERVATION_BYTES,
        ) else {
            result.rejections.push(reject(
                identity,
                "observation_too_large",
                provider_labels::MAX_OBSERVATION_BYTES + 1,
            ));
            continue;
        };
        let mut metadata = serde_json::Map::new();
        for (key, value) in borrowed {
            metadata.insert(key.into(), value.clone());
        }
        if !borrowed_source.is_empty() {
            metadata.insert(
                "source".into(),
                serde_json::Value::Object(
                    borrowed_source
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value.clone()))
                        .collect(),
                ),
            );
        } else if let Some(source) = source_text {
            metadata.insert("source".into(), source.clone());
        }
        let normalized_host = url::Url::parse(exact_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
        metadata.insert("normalized_url_host".into(), normalized_host.into());
        let raw_metadata = canonical_capture_value(&serde_json::Value::Object(metadata));
        let categories = object
            .get("category")
            .or_else(|| object.get("categories"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let categories = canonical_capture_value(&categories);
        let categories_hash = crate::classifier::artifact::sha256_hex(
            &serde_json::to_vec(&categories).expect("bounded JSON value serializes"),
        );
        let title = exact_text(&["title"]);
        let description = exact_text(&["description"]);
        let language = object
            .get("language")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let input = crate::classifier::ClassifyInput::new(
            "provider-observation",
            title,
            description,
            language.clone(),
        );
        let format_key = serde_json::to_string(&actual).expect("provider format serializes");
        let observation_id = crate::classifier::artifact::sha256_hex(
            crate::storage::codec::encode(&[
                "classifier-provider-observation-v1",
                &format_key,
                &sanitized,
                identity,
                &input.input_hash,
                &categories_hash,
            ])
            .as_bytes(),
        );
        let source_revision = crate::classifier::artifact::sha256_hex(
            &serde_json::to_vec(&raw_metadata).expect("bounded metadata serializes"),
        );
        let publisher = object
            .get("source")
            .and_then(|source| source.get("name"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| object.get("source").and_then(serde_json::Value::as_str))
            .or_else(|| {
                object
                    .get("source_name")
                    .and_then(serde_json::Value::as_str)
            })
            .or_else(|| object.get("source_id").and_then(serde_json::Value::as_str))
            .map(str::to_owned);
        let observation = ProviderObservation {
            observation_id,
            provider: actual.clone(),
            endpoint: sanitized.clone(),
            publisher,
            url: exact_url.into(),
            article_id: (!supplied_id.is_empty()).then(|| supplied_id.into()),
            published_at: object
                .get("pubDate")
                .or_else(|| object.get("published"))
                .or_else(|| object.get("publishedAt"))
                .or_else(|| object.get("published_at"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            fetched_at: context.fetched_at.clone(),
            cycle_ids: vec![context.cycle_id.clone()],
            input_hash: input.input_hash,
            source_revision,
            title: title.into(),
            description: description.into(),
            language,
            raw_categories: categories,
            raw_ai_tags: object
                .get("ai_tag")
                .or_else(|| object.get("ai_tags"))
                .map(canonical_capture_value),
            raw_metadata,
        };
        if provider_labels::validate_observation(&observation).is_err() {
            result
                .rejections
                .push(reject(identity, "observation_too_large", bytes));
            continue;
        }
        if object
            .keys()
            .any(|key| !CAPTURE_FIELDS.contains(&key.as_str()))
        {
            result.rejections.push(reject(
                identity,
                "unallowlisted_article_metadata_excluded",
                0,
            ));
        }
        result.observations.push(observation);
    }
    result
}
fn capture_depth_bounded<'a>(values: impl IntoIterator<Item = &'a serde_json::Value>) -> bool {
    fn visit(value: &serde_json::Value, depth: usize, remaining: &mut usize) -> bool {
        if depth > 64 || *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        match value {
            serde_json::Value::Array(values) => values
                .iter()
                .all(|value| visit(value, depth + 1, remaining)),
            serde_json::Value::Object(values) => values
                .values()
                .all(|value| visit(value, depth + 1, remaining)),
            _ => true,
        }
    }
    let mut remaining = crate::classifier::provider_labels::MAX_OBSERVATION_BYTES;
    values
        .into_iter()
        .all(|value| visit(value, 0, &mut remaining))
}

fn capture_json_size(value: &impl serde::Serialize, limit: usize) -> std::io::Result<usize> {
    struct Bound {
        limit: usize,
        written: usize,
    }
    impl std::io::Write for Bound {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.written = self
                .written
                .checked_add(bytes.len())
                .filter(|bytes| *bytes <= self.limit)
                .ok_or_else(|| std::io::Error::other("capture bound"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut bound = Bound { limit, written: 0 };
    serde_json::to_writer(&mut bound, value).map_err(std::io::Error::other)?;
    Ok(bound.written)
}
fn canonical_capture_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut sorted: Vec<_> = object.iter().collect();
            sorted.sort_by(|left, right| left.0.cmp(right.0));
            serde_json::Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_capture_value(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_capture_value).collect())
        }
        value => value.clone(),
    }
}
