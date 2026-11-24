use std::env;
use std::io;
use std::path::Path;
use std::time::Instant;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use rust_bert::pipelines::common::ModelResource;
use rust_bert::pipelines::common::ModelType;
use rust_bert::pipelines::generation_utils::GenerateConfig;
use rust_bert::pipelines::generation_utils::LanguageGenerator;
use rust_bert::resources::LocalResource;
use rust_bert::resources::ResourceProvider;
use rust_bert::t5::T5Generator;
use serde::Deserialize;
use serde_json::json;
use tch::Device;
use tch::Tensor;

const MAX_INPUT_TOKENS: usize = 256;
const MAX_OUTPUT_TOKENS: i64 = 48;
const MAX_CASES: usize = 16;

#[derive(Deserialize)]
struct Prompt {
    id: String,
    text: String,
}

fn main() -> Result<()> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    ensure!(
        arguments.len() == 2 && arguments[0] == "t5",
        "expected t5 and a model directory"
    );
    let dir = Path::new(&arguments[1]);
    let prompts: Vec<Prompt> = serde_json::from_reader(io::stdin().lock())?;
    ensure!(
        (1..=MAX_CASES).contains(&prompts.len()),
        "invalid case count"
    );
    ensure!(
        prompts.iter().all(|prompt| prompt.text.len() <= 2048),
        "case exceeds the input byte limit"
    );
    tch::set_num_threads(1);
    tch::set_num_interop_threads(1);
    let started = Instant::now();

    run(
        T5Generator::new(config(dir)).context("load T5 checkpoint")?,
        &prompts,
        started,
    )
}

fn config(dir: &Path) -> GenerateConfig {
    let resource = |name| -> Box<dyn ResourceProvider + Send> {
        Box::new(LocalResource {
            local_path: dir.join(name),
        })
    };
    GenerateConfig {
        model_type: ModelType::T5,
        model_resource: ModelResource::Torch(resource("model.safetensors")),
        config_resource: resource("config.json"),
        vocab_resource: resource("spiece.model"),
        merges_resource: None,
        min_length: 0,
        max_length: Some(MAX_OUTPUT_TOKENS + 1),
        do_sample: false,
        early_stopping: false,
        num_beams: 1,
        temperature: 1.0,
        top_k: 0,
        top_p: 1.0,
        repetition_penalty: 1.0,
        length_penalty: 1.0,
        no_repeat_ngram_size: 0,
        num_return_sequences: 1,
        num_beam_groups: None,
        diversity_penalty: None,
        device: Device::Cpu,
        kind: None,
    }
}

fn run(generator: impl LanguageGenerator, prompts: &[Prompt], started: Instant) -> Result<()> {
    println!(
        "{}",
        json!({"model_load_ms": started.elapsed().as_millis()})
    );

    let tokenizer = generator.get_tokenizer();
    for prompt in prompts {
        let started = Instant::now();
        let tokens = tokenizer.tokenize_with_offsets(&prompt.text).tokens;
        let mut ids = tokenizer.convert_tokens_to_ids(&tokens);
        let eos = tokenizer.get_eos_id().context("T5 tokenizer lacks EOS")?;
        if ids.last() != Some(&eos) {
            ids.push(eos);
        }
        ensure!(
            (1..=MAX_INPUT_TOKENS).contains(&ids.len()),
            "case {} exceeds token bounds",
            prompt.id
        );
        let input = Tensor::f_from_slice(&ids)?.f_unsqueeze(0)?;
        let outputs = generator.generate_from_ids_and_past(input, None, None)?;
        let output = outputs.first().context("no generation returned")?;
        // The first token is T5's decoder start token.
        let reply = output
            .indices
            .get(1..)
            .context("T5 omitted decoder start")?;
        let answer = tokenizer.decode(reply, true, false);
        println!(
            "{}",
            json!({
                "id": prompt.id,
                "latency_ms": started.elapsed().as_millis(),
                "input_tokens": ids.len(),
                "output_tokens": reply.len(),
                "answer": answer.trim(),
            })
        );
    }
    Ok(())
}
