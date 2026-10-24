//! Canonical identities, bounded caches and immutable evidence provenance.
use super::*;
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
