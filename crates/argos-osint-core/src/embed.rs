//! Local sentence embeddings for Brain recall.
//!
//! Model: Xenova's quantized `all-MiniLM-L6-v2` ONNX export (384-d), run on the CPU
//! with tract-onnx (pure Rust, so it builds on Intel Macs where ort 2 / FastEmbed
//! does not). Tokenization uses the model's own `tokenizer.json` through the
//! `tokenizers` crate (no `onig` C dependency). Sentence vectors are mean-pooled over
//! the attention mask and L2-normalized, matching sentence-transformers.
//!
//! Files live in [`model_dir`] (`ARGOS_HOME/models/all-MiniLM-L6-v2` unless
//! `ARGOS_EMBED_MODEL_DIR` points elsewhere). Missing files are downloaded from
//! Hugging Face on first use (~23 MB model + ~0.7 MB tokenizer). To install offline,
//! put `model_quantized.onnx` and `tokenizer.json` in that directory yourself.
//!
//! `ARGOS_EMBED=0` (or `false`/`off`/`no`) disables embedding: Brain recall then
//! degrades to Jaccard. Unit tests in this crate default to disabled so the default
//! `cargo test` never touches the network; `ARGOS_EMBED=1` opts in.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use tract_onnx::prelude::*;

/// Vector width of all-MiniLM-L6-v2.
pub const DIM: usize = 384;
pub const MODEL_REPO: &str = "Xenova/all-MiniLM-L6-v2";
pub const MODEL_FILE: &str = "model_quantized.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
/// all-MiniLM-L6-v2 was trained on 256 word pieces; longer text is truncated.
const MAX_TOKENS: usize = 256;
const DOWNLOAD_TIMEOUT_SECS: u64 = 180;

/// False when `ARGOS_EMBED` is `0`, `false`, `off`, or `no`. Unset means enabled,
/// except in this crate's unit tests, which stay offline unless `ARGOS_EMBED=1`.
pub fn enabled() -> bool {
    #[cfg(test)]
    if testing::active() {
        return true;
    }
    flag_enabled(std::env::var("ARGOS_EMBED").ok().as_deref())
}

fn flag_enabled(value: Option<&str>) -> bool {
    match value {
        Some(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        None => !cfg!(test),
    }
}

/// Identifies the model, pooling, and width that produced stored vectors. A change
/// here makes the Brain index rebuild itself from SQLite.
pub fn fingerprint() -> String {
    #[cfg(test)]
    if testing::active() {
        return format!("test-hash;dim={DIM}");
    }
    format!("{MODEL_REPO}/onnx/{MODEL_FILE};dim={DIM};pool=mean;norm=l2;max_tokens={MAX_TOKENS}")
}

/// Where the model and tokenizer are cached.
pub fn model_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ARGOS_EMBED_MODEL_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    crate::paths::home_dir()
        .join("models")
        .join("all-MiniLM-L6-v2")
}

type Plan = Arc<TypedRunnableModel>;

struct Embedder {
    tokenizer: tokenizers::Tokenizer,
    plan: Plan,
    /// Model input index for each of input_ids, attention_mask, token_type_ids.
    order: [Option<usize>; 3],
    inputs: usize,
}

static EMBEDDER: OnceLock<std::result::Result<Embedder, String>> = OnceLock::new();

fn embedder() -> Result<&'static Embedder> {
    anyhow::ensure!(enabled(), "embedding disabled (ARGOS_EMBED=0)");
    EMBEDDER
        .get_or_init(|| load(&model_dir()).map_err(|err| format!("{err:#}")))
        .as_ref()
        .map_err(|err| anyhow!("embedding model unavailable: {err}"))
}

/// Loads (downloading first if needed) the model. Exposed so `argos memories reindex`
/// can report a clear error before touching the index.
pub fn warm_up() -> Result<()> {
    #[cfg(test)]
    if testing::active() {
        return Ok(());
    }
    embedder().map(|_| ())
}

/// One normalized 384-d vector.
pub fn embed_one(text: &str) -> Result<Vec<f32>> {
    #[cfg(test)]
    if testing::active() {
        return Ok(testing::hash_embed(text));
    }
    embedder()?.embed(text)
}

pub fn embed_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    #[cfg(test)]
    if testing::active() {
        return Ok(texts.iter().map(|text| testing::hash_embed(text)).collect());
    }
    let model = embedder()?;
    texts.iter().map(|text| model.embed(text)).collect()
}

fn load(dir: &Path) -> Result<Embedder> {
    let model_path = dir.join(MODEL_FILE);
    let tokenizer_path = dir.join(TOKENIZER_FILE);
    ensure_file(&tokenizer_path, &hf_url(TOKENIZER_FILE))?;
    ensure_file(&model_path, &hf_url(&format!("onnx/{MODEL_FILE}")))?;
    let mut tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
        .map_err(|err| anyhow!("load {}: {err}", tokenizer_path.display()))?;
    tokenizer
        .with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_TOKENS,
            ..Default::default()
        }))
        .map_err(|err| anyhow!("tokenizer truncation: {err}"))?;
    tokenizer.with_padding(None);

    // The export declares i64 inputs of shape [batch_size, sequence_length] with
    // symbolic dims; keep them symbolic so any sentence length runs.
    let model = tract_onnx::onnx()
        .model_for_path(&model_path)
        .with_context(|| format!("load {}", model_path.display()))?;
    let names: Vec<String> = model
        .input_outlets()?
        .iter()
        .map(|outlet| model.node(outlet.node).name.clone())
        .collect();
    let mut order = [None; 3];
    for (index, name) in names.iter().enumerate() {
        let slot = match name.as_str() {
            "input_ids" => 0,
            "attention_mask" => 1,
            "token_type_ids" => 2,
            other => anyhow::bail!("unexpected model input {other}"),
        };
        order[slot] = Some(index);
    }
    anyhow::ensure!(order[0].is_some(), "model has no input_ids input");
    let plan = model.into_optimized()?.into_runnable()?;
    Ok(Embedder {
        tokenizer,
        plan,
        order,
        inputs: names.len(),
    })
}

impl Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|err| anyhow!("tokenize: {err}"))?;
        let len = encoding.get_ids().len();
        anyhow::ensure!(len > 0, "tokenizer produced no tokens");
        let as_tensor = |values: Vec<i64>| -> Result<Tensor> {
            Ok(tract_ndarray::Array2::from_shape_vec((1, len), values)?.into())
        };
        let columns = [
            encoding.get_ids().iter().map(|v| i64::from(*v)).collect::<Vec<_>>(),
            encoding
                .get_attention_mask()
                .iter()
                .map(|v| i64::from(*v))
                .collect(),
            encoding.get_type_ids().iter().map(|v| i64::from(*v)).collect(),
        ];
        let mut inputs: Vec<Option<TValue>> = vec![None; self.inputs];
        for (slot, values) in columns.into_iter().enumerate() {
            if let Some(index) = self.order[slot] {
                inputs[index] = Some(as_tensor(values)?.into());
            }
        }
        let inputs: TVec<TValue> = inputs
            .into_iter()
            .map(|value| value.ok_or_else(|| anyhow!("missing model input")))
            .collect::<Result<_>>()?;
        let outputs = self.plan.run(inputs)?;
        let hidden = outputs[0].to_plain_array_view::<f32>()?;
        anyhow::ensure!(
            hidden.ndim() == 3 && hidden.shape()[2] == DIM,
            "unexpected model output shape {:?}",
            hidden.shape()
        );
        let mask = encoding.get_attention_mask();
        let mut pooled = vec![0f32; DIM];
        let mut count = 0f32;
        for (token, weight) in mask.iter().enumerate() {
            if *weight == 0 {
                continue;
            }
            count += 1.0;
            for (dim, slot) in pooled.iter_mut().enumerate() {
                *slot += hidden[[0, token, dim]];
            }
        }
        anyhow::ensure!(count > 0.0, "empty attention mask");
        for slot in &mut pooled {
            *slot /= count;
        }
        Ok(normalize(pooled))
    }
}

pub(crate) fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn hf_url(file: &str) -> String {
    format!("https://huggingface.co/{MODEL_REPO}/resolve/main/{file}")
}

/// Downloads `url` to `path` unless it already exists. Writes to a temp file first so
/// an interrupted download is never mistaken for a model.
fn ensure_file(path: &Path, url: &str) -> Result<()> {
    if path.is_file() && std::fs::metadata(path)?.len() > 0 {
        return Ok(());
    }
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("no parent for {}", path.display()))?;
    std::fs::create_dir_all(dir)?;
    let url = url.to_string();
    let bytes = crate::brain_lance::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
            .user_agent("argos-osint (brain embeddings)")
            .build()?;
        let response = client.get(&url).send().await?.error_for_status()?;
        anyhow::Ok(response.bytes().await?.to_vec())
    })
    .with_context(|| format!("download {} (set ARGOS_EMBED=0 to skip embeddings)", path.display()))?;
    anyhow::ensure!(!bytes.is_empty(), "empty download for {}", path.display());
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Offline stand-in for unit tests: a hashed bag of words, so Store and Lance paths
/// run without the model. Per thread, so parallel tests do not see each other.
#[cfg(test)]
pub(crate) mod testing {
    use std::cell::Cell;

    thread_local! {
        static FAKE: Cell<bool> = const { Cell::new(false) };
    }

    pub(crate) fn active() -> bool {
        FAKE.with(Cell::get)
    }

    /// Turns the fake embedder on for this thread until the guard drops.
    pub(crate) fn fake() -> Guard {
        FAKE.with(|flag| flag.set(true));
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            FAKE.with(|flag| flag.set(false));
        }
    }

    pub(crate) fn hash_embed(text: &str) -> Vec<f32> {
        let mut vector = vec![0f32; super::DIM];
        for token in crate::brain::tokenize(text) {
            let mut hash: u64 = 0xcbf29ce484222325;
            for byte in token.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
            vector[(hash % super::DIM as u64) as usize] += 1.0;
        }
        if vector.iter().all(|v| *v == 0.0) {
            vector[0] = 1.0;
        }
        super::normalize(vector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_makes_unit_vectors_and_keeps_zero() {
        let v = normalize(vec![3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        assert_eq!(normalize(vec![0.0, 0.0]), vec![0.0, 0.0]);
        assert!(fingerprint().contains("dim=384"));
    }

    #[test]
    fn argos_embed_zero_disables_and_tests_default_offline() {
        assert!(!flag_enabled(Some("0")));
        assert!(!flag_enabled(Some(" off ")));
        assert!(!flag_enabled(Some("FALSE")));
        assert!(flag_enabled(Some("1")));
        assert!(!flag_enabled(None), "unit tests stay offline unless ARGOS_EMBED=1");
    }

    /// Downloads the model; run with `ARGOS_EMBED=1 cargo test -- --ignored`.
    #[test]
    #[ignore = "downloads all-MiniLM-L6-v2; set ARGOS_EMBED=1 and pass --ignored"]
    fn minilm_ranks_paraphrase_above_unrelated() {
        if !enabled() {
            eprintln!("skipped: ARGOS_EMBED is not enabled");
            return;
        }
        let a = embed_one("The ship docked at the harbor at dawn").unwrap();
        let b = embed_one("At sunrise the vessel arrived in port").unwrap();
        let c = embed_one("Quarterly tax filings are due in April").unwrap();
        assert_eq!(a.len(), DIM);
        let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>();
        assert!(dot(&a, &b) > dot(&a, &c) + 0.2, "{} vs {}", dot(&a, &b), dot(&a, &c));
    }
}
