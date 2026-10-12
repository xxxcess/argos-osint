//! Canonical identities, bounded caches and immutable evidence provenance.
use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::VecDeque;
pub fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
pub fn canonical(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(o) => {
            let mut keys: Vec<_> = o.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonical(&o[k]));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(a) => serde_json::Value::Array(a.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
pub fn key(value: &serde_json::Value) -> String {
    hash(&canonical(value).to_string())
}
pub fn feature_key(source: &Source, context: &ParseContext) -> String {
    key(
        &serde_json::json!({"prose_hash":source.prose_hash,"schema":SCHEMA,"context":context,
        "core":"61e2541597f27c592947e5059c083548f24aa6fba14cf2b109a00db44de6ca57",
        "dates":"3bfc87cf5c0fc6d4190c195078aa969675e9de32a70836cb233d87b724d6aaf2",
        "bridge":hash(include_str!("bridge.js")),"lexicon":"static-v1","chunks":"paragraph-4096-8192-overlap-v1","spans":"utf16-utf8-v1"}),
    )
}
#[derive(Default)]
pub(crate) struct MemoryCache {
    entries: VecDeque<(Annotation, usize)>,
    bytes: usize,
}
impl MemoryCache {
    pub fn get(&mut self, key: &str) -> Option<Annotation> {
        let i = self
            .entries
            .iter()
            .position(|(a, _)| a.feature_key == key)?;
        let entry = self.entries.remove(i)?;
        let out = entry.0.clone();
        self.entries.push_back(entry);
        Some(out)
    }
    pub fn put(&mut self, a: Annotation) {
        let size = serde_json::to_vec(&a)
            .map(|v| v.len())
            .unwrap_or(MAX_OUTPUT);
        if size > 64 * 1024 * 1024 {
            return;
        }
        while self.bytes + size > 64 * 1024 * 1024 {
            if let Some((_, n)) = self.entries.pop_front() {
                self.bytes -= n;
            } else {
                break;
            }
        }
        self.bytes += size;
        self.entries.push_back((a, size));
    }
}
pub fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(argos_derived_summaries)")?
        .query_map([], |r| r.get(1))?
        .collect::<rusqlite::Result<_>>()?;
    if !columns.iter().any(|c| c == "result_json") {
        conn.execute_batch("ALTER TABLE argos_derived_summaries ADD COLUMN result_json TEXT NOT NULL DEFAULT '{}';")?;
    }

    conn.execute_batch("DROP INDEX IF EXISTS argos_derived_summaries_key;
    CREATE TABLE IF NOT EXISTS nlp_artifacts(hash TEXT PRIMARY KEY,text TEXT NOT NULL,byte_len INTEGER NOT NULL,created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS nlp_source_refs(ref_key TEXT PRIMARY KEY,owner_kind TEXT NOT NULL,owner_id TEXT NOT NULL,source_id TEXT NOT NULL,revision TEXT NOT NULL,attempt_id TEXT NOT NULL,raw_hash TEXT,prose_hash TEXT NOT NULL,metadata_json TEXT NOT NULL,transform_version TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS nlp_ref_owner ON nlp_source_refs(owner_kind,owner_id);
    CREATE TABLE IF NOT EXISTS nlp_features(feature_key TEXT PRIMARY KEY,prose_hash TEXT NOT NULL,version_context_json TEXT NOT NULL,annotations_json TEXT NOT NULL,byte_len INTEGER NOT NULL,last_access TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS nlp_feature_lru ON nlp_features(last_access);")?;
    Ok(())
}
pub fn artifact(conn: &Connection, text: &str) -> anyhow::Result<String> {
    let hash = hash(text);
    conn.execute(
        "INSERT OR IGNORE INTO nlp_artifacts(hash,text,byte_len,created_at) VALUES(?1,?2,?3,?4)",
        params![
            hash,
            text,
            text.len() as i64,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(hash)
}
pub fn register(
    conn: &Connection,
    owner_kind: &str,
    owner_id: &str,
    source: &Source,
    raw: Option<&str>,
    transform: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        hash(&source.prose) == source.prose_hash,
        "prose hash mismatch"
    );
    let raw_hash = raw.map(|s| artifact(conn, s)).transpose()?;
    anyhow::ensure!(raw_hash == source.raw_hash, "raw provenance mismatch");
    artifact(conn, &source.prose)?;
    let ref_key = key(&serde_json::json!([
        owner_kind,
        owner_id,
        source.source_id,
        source.revision,
        source.attempt_id,
        raw_hash,
        source.prose_hash,
        transform
    ]));
    conn.execute("INSERT OR IGNORE INTO nlp_source_refs VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![ref_key,owner_kind,owner_id,source.source_id,source.revision,source.attempt_id,raw_hash,source.prose_hash,serde_json::json!({"language":source.language_hint,"published_at":source.published_at,"retrieved_at":source.retrieved_at,"historical_derived":raw.is_none()}).to_string(),transform])?;
    Ok(())
}
pub fn get(
    conn: &Connection,
    source: &Source,
    context: &ParseContext,
) -> anyhow::Result<Option<Annotation>> {
    let key = feature_key(source, context);
    let json: Option<String> = conn
        .query_row(
            "SELECT annotations_json FROM nlp_features WHERE feature_key=?1",
            [&key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(json) = json else { return Ok(None) };
    let a: Annotation = serde_json::from_str(&json)?;
    if a.status != Status::Complete || a.feature_key != key || !a.validate(source) {
        return Ok(None);
    }
    conn.execute(
        "UPDATE nlp_features SET last_access=?2 WHERE feature_key=?1",
        params![key, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(Some(a))
}
pub fn put(
    conn: &Connection,
    source: &Source,
    context: &ParseContext,
    a: &Annotation,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        a.status == Status::Complete
            && a.validate(source)
            && a.feature_key == feature_key(source, context),
        "invalid NLP cache entry"
    );
    let json = serde_json::to_string(a)?;
    conn.execute(
        "INSERT OR REPLACE INTO nlp_features VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            a.feature_key,
            a.prose_hash,
            serde_json::json!({"context":context,"schema":SCHEMA,"bridge_hash":hash(include_str!("bridge.js")),"policy":"paragraph-4096-8192-overlap-v1"}).to_string(),
            json,
            json.len() as i64,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    loop {
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(byte_len),0) FROM nlp_features",
            [],
            |r| r.get(0),
        )?;
        if total <= 256 * 1024 * 1024 {
            break;
        }
        conn.execute("DELETE FROM nlp_features WHERE feature_key=(SELECT feature_key FROM nlp_features ORDER BY last_access,feature_key LIMIT 1)",[])?;
    }
    Ok(())
}
pub fn remove_owner(conn: &Connection, kind: &str, id: &str) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM nlp_source_refs WHERE owner_kind=?1 AND owner_id=?2",
        params![kind, id],
    )?;
    gc(conn)
}
pub fn gc(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("DELETE FROM nlp_features WHERE prose_hash NOT IN(SELECT prose_hash FROM nlp_source_refs);
    DELETE FROM nlp_artifacts WHERE hash NOT IN(SELECT prose_hash FROM nlp_source_refs UNION SELECT raw_hash FROM nlp_source_refs WHERE raw_hash IS NOT NULL UNION SELECT json_extract(evidence_json,'$.artifact_hash') FROM insight_derivation_evidence UNION SELECT json_extract(value,'$.artifact_hash') FROM insight_snapshots,json_each(insight_snapshots.inputs_json,'$.source_refs'));")?;
    Ok(())
}
