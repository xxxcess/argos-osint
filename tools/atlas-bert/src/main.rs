use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::Read;
use std::io::Write;
use std::path::Path;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use rust_bert::pipelines::common::ModelResource;
use rust_bert::pipelines::common::ModelType;
use rust_bert::pipelines::zero_shot_classification::ZeroShotClassificationConfig;
use rust_bert::pipelines::zero_shot_classification::ZeroShotClassificationModel;
use rust_bert::resources::LocalResource;
use serde::Deserialize;
use serde::Serialize;
use tch::Device;

const PROTOCOL: &str = "argos-atlas-bert-v1";
const MAX_BATCH: usize = 4;
const MAX_TEXT_BYTES: usize = 32 * 1024 + 1;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_TOKENS: usize = 192;

// Frozen v1 hypotheses follow Atlas DomainId::ALL and DomainId::definition.
// The order and text are part of the model's score and calibration contract.
const DOMAINS: [&str; 6] = [
    "Institutions, elections, authority and diplomacy.",
    "Armed threats, crime, cyber incidents and protective actions.",
    "Finance, trade, production, logistics and supply resilience.",
    "Health, displacement, food access and essential services.",
    "Hazards, pollution, water, ecosystems and natural resources.",
    "Research, technical performance and scientific or technological capability.",
];

#[derive(Deserialize)]
struct CheckpointConfig {
    model_type: String,
    id2label: BTreeMap<usize, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    texts: Vec<String>,
}

#[derive(Serialize)]
struct Ready {
    protocol: &'static str,
}

#[derive(Serialize)]
struct Response {
    scores: Vec<[f64; DOMAINS.len()]>,
}

#[derive(Debug, PartialEq, Eq)]
enum LabelOrder {
    EntailmentLast,
    EntailmentFirst,
}

impl CheckpointConfig {
    fn label_order(&self) -> Result<LabelOrder> {
        ensure!(
            self.model_type == "distilbert",
            "Atlas BERT requires a DistilBERT checkpoint"
        );
        ensure!(
            self.id2label.len() == 3
                && self
                    .id2label
                    .get(&1)
                    .is_some_and(|label| label.eq_ignore_ascii_case("neutral")),
            "Atlas BERT requires three MNLI labels with neutral at index 1"
        );
        let label_is = |index, expected| {
            self.id2label
                .get(&index)
                .is_some_and(|label| label.eq_ignore_ascii_case(expected))
        };
        if label_is(0, "contradiction") && label_is(2, "entailment") {
            Ok(LabelOrder::EntailmentLast)
        } else if label_is(0, "entailment") && label_is(2, "contradiction") {
            Ok(LabelOrder::EntailmentFirst)
        } else {
            Err(anyhow!(
                "Atlas BERT requires contradiction and entailment at the MNLI endpoints"
            ))
        }
    }
}

impl LabelOrder {
    fn entailment_score(&self, last_score: f64) -> f64 {
        // rust-bert normalizes endpoints 0 and 2 and returns the score for 2.
        match self {
            Self::EntailmentLast => last_score,
            Self::EntailmentFirst => 1.0 - last_score,
        }
    }
}

fn local_resource(model_dir: &Path, name: &str) -> Result<LocalResource> {
    let local_path = model_dir.join(name);
    let metadata =
        fs::metadata(&local_path).with_context(|| format!("read model resource {name}"))?;
    ensure!(
        metadata.is_file() && metadata.len() > 0 && local_path.to_str().is_some(),
        "invalid model resource {name}"
    );
    Ok(LocalResource { local_path })
}

fn load_model(model_dir: &Path) -> Result<(ZeroShotClassificationModel, LabelOrder)> {
    let config = local_resource(model_dir, "config.json")?;
    let vocab = local_resource(model_dir, "vocab.txt")?;
    let weights = local_resource(model_dir, "rust_model.ot")?;
    let mut bytes = Vec::new();
    File::open(&config.local_path)?
        .take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_CONFIG_BYTES, "model config is too large");
    let order = serde_json::from_slice::<CheckpointConfig>(&bytes)
        .context("read DistilBERT model config")?
        .label_order()?;

    tch::set_num_threads(1);
    tch::set_num_interop_threads(1);
    let model = ZeroShotClassificationModel::new(ZeroShotClassificationConfig {
        model_type: ModelType::DistilBert,
        model_resource: ModelResource::Torch(Box::new(weights)),
        config_resource: Box::new(config),
        vocab_resource: Box::new(vocab),
        merges_resource: None,
        lower_case: true,
        strip_accents: None,
        add_prefix_space: None,
        device: Device::Cpu,
        kind: None,
    })
    .context("load local DistilBERT MNLI checkpoint")?;
    Ok((model, order))
}

fn read_request(reader: &mut impl BufRead) -> Result<Option<Request>> {
    let mut line = Vec::new();
    let size = reader
        .take((MAX_LINE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if size == 0 {
        return Ok(None);
    }
    ensure!(
        size <= MAX_LINE_BYTES && line.ends_with(b"\n"),
        "oversized or unterminated Atlas BERT request"
    );
    let request: Request =
        serde_json::from_slice(&line).map_err(|_| anyhow!("invalid Atlas BERT request"))?;
    ensure!(
        (1..=MAX_BATCH).contains(&request.texts.len())
            && request
                .texts
                .iter()
                .all(|text| text.len() <= MAX_TEXT_BYTES && !text.trim().is_empty()),
        "invalid Atlas BERT input count or text length"
    );
    Ok(Some(request))
}

fn write_line(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn score(
    model: &ZeroShotClassificationModel,
    label_order: &LabelOrder,
    request: &Request,
) -> Result<Response> {
    let texts: Vec<&str> = request.texts.iter().map(String::as_str).collect();
    let predictions = model
        .predict_multilabel(
            &texts,
            &DOMAINS,
            Some(Box::new(|scope| {
                format!("This article substantially discusses the following topic: {scope}")
            })),
            MAX_TOKENS,
        )
        .map_err(|_| anyhow!("Atlas BERT inference failed"))?;
    ensure!(
        predictions.len() == texts.len(),
        "invalid Atlas BERT batch output"
    );
    let mut scores = Vec::with_capacity(texts.len());
    for (row, labels) in predictions.into_iter().enumerate() {
        ensure!(
            labels.len() == DOMAINS.len(),
            "invalid Atlas BERT domain output"
        );
        let mut values = [0.0; DOMAINS.len()];
        for (index, label) in labels.into_iter().enumerate() {
            ensure!(
                label.sentence == row
                    && label.id == index as i64
                    && label.text == DOMAINS[index]
                    && (0.0..=1.0).contains(&label.score),
                "invalid Atlas BERT domain score"
            );
            values[index] = label_order.entailment_score(label.score);
        }
        scores.push(values);
    }
    Ok(Response { scores })
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let (Some(command), Some(model_dir), None) = (args.next(), args.next(), args.next()) else {
        return Err(anyhow!("usage: argos-atlas-bert serve <model-dir>"));
    };
    ensure!(
        command == "serve",
        "usage: argos-atlas-bert serve <model-dir>"
    );

    let (model, order) = load_model(Path::new(&model_dir))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    write_line(&mut output, &Ready { protocol: PROTOCOL })?;
    while let Some(request) = read_request(&mut input)? {
        write_line(&mut output, &score(&model, &order, &request)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typeform_label_order_uses_the_entailment_endpoint() {
        let config: CheckpointConfig = serde_json::from_str(
            r#"{"model_type":"distilbert","id2label":{"0":"ENTAILMENT","1":"NEUTRAL","2":"CONTRADICTION"}}"#,
        )
        .unwrap();
        let order = config.label_order().unwrap();
        assert_eq!(order, LabelOrder::EntailmentFirst);
        assert_eq!(order.entailment_score(0.25), 0.75);
    }

    #[test]
    fn unexpected_label_order_is_rejected() {
        let config: CheckpointConfig = serde_json::from_str(
            r#"{"model_type":"distilbert","id2label":{"0":"contradiction","1":"entailment","2":"neutral"}}"#,
        )
        .unwrap();
        assert!(config.label_order().is_err());
    }

    #[test]
    fn request_requires_a_bounded_complete_batch() {
        let input = b"{\"texts\":[\"An article\"]}\n";
        let mut reader = input.as_slice();
        assert_eq!(
            read_request(&mut reader).unwrap().unwrap().texts,
            ["An article"]
        );
        assert!(read_request(&mut reader).unwrap().is_none());

        for mut bad in [
            b"{\"texts\":[\"unterminated\"]}".as_slice(),
            b"{\"texts\":[]}\n",
            b"{\"texts\":[\"a\",\"b\",\"c\",\"d\",\"e\"]}\n",
            b"{\"texts\":[\" \"]}\n",
        ] {
            assert!(read_request(&mut bad).is_err());
        }
        let mut oversized = vec![b'x'; MAX_LINE_BYTES + 1];
        oversized.push(b'\n');
        assert!(read_request(&mut oversized.as_slice()).is_err());
    }
}
