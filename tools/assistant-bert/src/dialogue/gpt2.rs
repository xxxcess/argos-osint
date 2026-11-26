use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::ensure;
use anyhow::Result;
use rust_bert::gpt2::GPT2LMHeadModel;
use rust_bert::gpt2::Gpt2Config;
use rust_bert::pipelines::common::ModelType;
use rust_bert::pipelines::common::TokenizerOption;
use rust_bert::pipelines::generation_utils::Cache;
use rust_bert::pipelines::generation_utils::LMModelOutput;
use tch::nn;
use tch::Device;
use tch::Kind;
use tch::Tensor;

use super::QaModelTarget;

pub(super) struct Model {
    model: GPT2LMHeadModel,
    pub(super) tokenizer: TokenizerOption,
    pub(super) eos: i64,
}

impl Model {
    pub(super) fn load(dir: &Path, target: QaModelTarget) -> Result<Self> {
        let vocab = dir.join("vocab.json");
        let merges = dir.join("merges.txt");
        let tokenizer = TokenizerOption::from_file(
            ModelType::GPT2,
            vocab.to_str().expect("verified UTF-8 model directory"),
            merges.to_str(),
            false,
            None,
            None,
        )?;
        let eos = tokenizer
            .get_eos_id()
            .filter(|&token| token == 50_256)
            .ok_or_else(|| anyhow::anyhow!("invalid GPT-2 tokenizer EOS"))?;
        let mut config: Gpt2Config = serde_json::from_slice(&fs::read(dir.join("config.json"))?)?;
        let dimensions = match target {
            QaModelTarget::Gpt2Small => (12, 12, 768),
            QaModelTarget::Gpt2Medium => (24, 16, 1024),
            _ => anyhow::bail!("invalid GPT-2 model target"),
        };
        ensure!(
            (config.n_layer, config.n_head, config.n_embd) == dimensions
                && config.vocab_size == 50_257
                && config.n_ctx == i64::from(target.input_tokens() + target.output_tokens())
                && config.n_positions == config.n_ctx,
            "unexpected GPT-2 checkpoint dimensions"
        );
        config.output_past = Some(true);

        // Decode the safetensors file before allocating the model so the file buffer and
        // newly allocated destination cannot both be resident with the decoded weights.
        let checkpoint = Tensor::read_safetensors(dir.join("model.safetensors"))?;
        let mut store = nn::VarStore::new(Device::Cpu);
        let model = GPT2LMHeadModel::new(store.root(), &config);
        load_official_weights(store.variables(), checkpoint)?;
        store.freeze();
        Ok(Self {
            model,
            tokenizer,
            eos,
        })
    }

    pub(super) fn forward(&self, input: &Tensor, cache: Cache) -> Result<LMModelOutput> {
        forward(&self.model, input, cache)
    }
}

fn forward(model: &GPT2LMHeadModel, input: &Tensor, cache: Cache) -> Result<LMModelOutput> {
    let past = match &cache {
        Cache::None => None,
        Cache::GPT2Cache(Some(past)) => Some(past),
        _ => anyhow::bail!("invalid GPT-2 decode cache"),
    };
    // The one-row prompt has no padding. Rust-BERT uses the cache length to set the
    // next position and applies its causal mask over the full sequence internally.
    Ok(model.forward_t(Some(input), past, None, None, None, None, false)?)
}

fn load_official_weights(
    variables: HashMap<String, Tensor>,
    checkpoint: Vec<(String, Tensor)>,
) -> Result<()> {
    // The original GPT-2 checkpoints use names such as h.0.attn.c_attn.weight.
    // Rust-BERT adds the transformer namespace and ties its output to wte.weight.
    let mut weights: HashMap<_, _> = checkpoint.into_iter().collect();
    ensure!(
        variables.len() == weights.len(),
        "unexpected GPT-2 tensor set"
    );
    tch::no_grad(|| {
        for (name, mut target) in variables {
            let name = name
                .strip_prefix("transformer.")
                .ok_or_else(|| anyhow::anyhow!("invalid GPT-2 variable namespace"))?;
            let source = weights
                .remove(name)
                .ok_or_else(|| anyhow::anyhow!("missing GPT-2 checkpoint tensor"))?;
            ensure!(
                source.kind() == Kind::Float && source.size() == target.size(),
                "unexpected GPT-2 checkpoint tensor type or shape"
            );
            target.f_copy_(&source)?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_official_tensor_names_and_requires_exact_shapes() {
        let mut store = nn::VarStore::new(Device::Cpu);
        let _ = (store.root() / "transformer" / "wte").zeros("weight", &[2, 3]);
        let weights = || {
            vec![(
                "wte.weight".into(),
                Tensor::ones([2, 3], (Kind::Float, Device::Cpu)),
            )]
        };
        load_official_weights(store.variables(), weights()).unwrap();
        store.freeze();
        let tensor = store.variables().remove("transformer.wte.weight").unwrap();
        assert_eq!(tensor.sum(Kind::Float).double_value(&[]), 6.0);

        let mut wrong_name = weights();
        wrong_name[0].0 = "wpe.weight".into();
        assert!(load_official_weights(store.variables(), wrong_name).is_err());
        let wrong_shape = Tensor::ones([1, 3], (Kind::Float, Device::Cpu));
        assert!(
            load_official_weights(store.variables(), vec![("wte.weight".into(), wrong_shape)])
                .is_err()
        );
    }

    #[test]
    fn cached_decoding_uses_the_same_position_as_the_full_prompt() {
        let store = nn::VarStore::new(Device::Cpu);
        let config = Gpt2Config {
            n_ctx: 8,
            n_positions: 8,
            n_layer: 1,
            n_head: 2,
            n_embd: 8,
            vocab_size: 16,
            output_past: Some(true),
            ..Gpt2Config::default()
        };
        let model = GPT2LMHeadModel::new(store.root(), &config);
        let input = |tokens: &[i64]| Tensor::from_slice(tokens).unsqueeze(0);
        tch::no_grad(|| {
            let prefix = forward(&model, &input(&[1, 4, 3]), Cache::None).unwrap();
            assert!(matches!(&prefix.cache, Cache::GPT2Cache(Some(_))));
            let cached = forward(&model, &input(&[2]), prefix.cache).unwrap();
            let full = forward(&model, &input(&[1, 4, 3, 2]), Cache::None).unwrap();
            assert!(
                cached.lm_logits.select(1, -1).allclose(
                    &full.lm_logits.select(1, -1),
                    1e-5,
                    1e-5,
                    false
                ),
                "one cached continuation should match the same token in a full decode"
            );
        });
    }
}
