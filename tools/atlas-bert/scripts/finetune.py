#!/usr/bin/env python3
"""Fit a pinned Atlas DistilBERT candidate and evaluate its frozen Rust assets."""
from __future__ import annotations

import argparse
from collections import Counter
import csv
from dataclasses import dataclass
import hashlib
import io
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib

from verify_linux import Commands, MODEL_COMMIT, ROOT, VerifyError, checkpoint_pins, sha

DATASET = ROOT / "datasets/argos_atlas_strategic_domains_v2.csv"
DATASET_SHA256 = "625f10150ba413f2fcae49ef425f9829004daaf9e69e49eecf66f7598de80d93"
DATASET_VERSION = "strategic-domains-v2-distinct-1"
SPLIT_COUNTS = {"train": 440, "development": 165, "calibration": 230, "test": 185}
DOMAINS = ("governance", "security", "economic", "human", "environmental", "scientific")
MODEL_FILES = ("config.json", "vocab.txt", "rust_model.ot")
FROZEN_NAME = "atlas-bert-frozen.json"
EVALUATION_NAME = "atlas-bert-evaluation.json"
BASELINE_THRESHOLD = .8
REGRESSION_POLICY = "atlas-bert-macro-f1-floor-v2"
MIN_PROMOTION_MACRO_F1 = 0.65
CALIBRATION_SCHEMA = "atlas-bert-calibration-v1"
THRESHOLD_GRID = range(50, 100)
SEED = 1701
EPOCHS = 3
TRAIN_BATCH = 24
LEARNING_RATE = 2e-5
TORCH_VERSION = "2.1.0"
TRANSFORMERS_VERSION = "4.38.2"


@dataclass(frozen=True)
class Article:
    text: str
    labels: frozenset[str]


def read_corpus():
    with DATASET.open("rb") as source:
        data = source.read(1024 * 1024 + 1)
    if hashlib.sha256(data).hexdigest() != DATASET_SHA256:
        raise VerifyError("The exact bundled 1,020-article corpus is required")
    return data


def read_articles(data, partitions):
    """Inspect article content and labels only for the phase's named partitions."""
    split = {name: [] for name in partitions}
    seen = set()
    for row in csv.DictReader(io.StringIO(data.decode("utf-8"), newline="")):
        rows = split.get(row["split"])
        if rows is None:
            continue
        labels = frozenset(filter(None, row["domain_labels"].split("|")))
        identity = row["scenario_group"]
        if (
            row["dataset_version"] != DATASET_VERSION
            or row["language"] != "en"
            or row["synthetic"] != "true"
            or row["labels_complete"] != "true"
            or len(row["title"].encode()) + len(row["description"].encode()) > 32 * 1024
            or not row["title"].strip()
            or not identity
            or identity in seen
            or not labels.issubset(DOMAINS)
        ):
            raise VerifyError("Pinned corpus partition metadata does not match the training contract")
        seen.add(identity)
        rows.append(Article(f"{row['title']}\n{row['description']}", labels))
    if any(len(rows) != SPLIT_COUNTS[name] for name, rows in split.items()):
        raise VerifyError("Pinned corpus partition counts do not match")
    return split


def runtime_contract():
    """Read the ordered hypothesis text and limits from the production worker."""
    main = (ROOT / "tools/atlas-bert/src/main.rs").read_text()
    taxonomy = (ROOT / "crates/argos-osint-core/src/atlas_taxonomy.rs").read_text()
    scope_list = re.search(r"const DOMAINS: \[&str; 6\] = \[(.*?)\];", main, re.S)
    limit = re.search(r"const MAX_TOKENS: usize = (\d+);", main)
    batch = re.search(r"const MAX_BATCH: usize = (\d+);", main)
    prefix = re.search(r'format!\("([^"]+)\{scope\}"\)', main)
    protocol = re.search(r'const PROTOCOL: &str = "([^"]+)";', main)
    domain_tokens = re.search(r"pub const ALL: \[DomainId; 6\] = \[(.*?)\];", taxonomy, re.S)
    if not all((scope_list, limit, batch, prefix, protocol, domain_tokens)):
        raise VerifyError("Production worker or taxonomy contract changed")
    scopes = json.loads("[" + scope_list[1].rstrip().removesuffix(",") + "]")
    ordered_domains = tuple(re.findall(r"DomainId::(\w+)", domain_tokens[1]))
    if tuple(name.lower() for name in ordered_domains) != DOMAINS or len(scopes) != len(DOMAINS):
        raise VerifyError("Production worker domain order changed")
    for token, scope in zip(ordered_domains, scopes, strict=True):
        definition = re.search(rf'Self::{token} =>\s*(?:\{{\s*)?"([^"]+)"', taxonomy)
        if not definition or definition[1] != scope:
            raise VerifyError("Production worker hypothesis and taxonomy scope differ")
    return {
        "protocol": protocol[1],
        "domains": list(DOMAINS),
        "hypotheses": [prefix[1] + scope for scope in scopes],
        "max_tokens": int(limit[1]),
        "max_batch": int(batch[1]),
    }


def model_assets(directory):
    result = {}
    for name in MODEL_FILES:
        path = directory / name
        if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 512 * 1024 * 1024:
            raise VerifyError("Model asset exceeds allowed size or is not a regular file")
        result[name] = sha(path)
    return result


def threshold(value):
    if type(value) not in (int, float) or not .5 <= value < 1.0:
        raise VerifyError("Admission threshold must be in [0.5, 1.0)")
    return value


def select_domains(scores, gate):
    # Match the runtime's finite log-odds conversion, eligibility and tie rules.
    calibrated = []
    for score in scores:
        bounded = min(1.0 - 1e-12, max(1e-12, score))
        log_odds = math.log(bounded) - math.log1p(-bounded)
        if log_odds >= 0:
            calibrated.append(1.0 / (1.0 + math.exp(-log_odds)))
        else:
            odds = math.exp(log_odds)
            calibrated.append(odds / (1.0 + odds))
    eligible = sorted(
        (index for index, score in enumerate(calibrated) if score >= gate),
        key=lambda index: (-calibrated[index], index),
    )
    if len(eligible) < 2:
        return tuple(DOMAINS[index] for index in eligible)
    first, second = eligible[:2]
    if calibrated[first] - calibrated[second] <= 1e-12:
        return (DOMAINS[first], DOMAINS[second]) if len(eligible) == 2 else ()
    if len(eligible) > 2 and calibrated[second] - calibrated[eligible[2]] <= 1e-12:
        return (DOMAINS[first],)
    return (DOMAINS[first], DOMAINS[second])


def f1(tp, fp, fn):
    denominator = 2 * tp + fp + fn
    return 2 * tp / denominator if denominator else 0.0


def metrics(articles, scores, gate):
    per_domain = {name: Counter(tp=0, fp=0, fn=0) for name in DOMAINS}
    false_out_of_scope = out_of_scope = 0
    top_two = Counter(
        pair_rows=0, exact_pair_rows=0, triad_rows=0, valid_triad_pairs=0,
        published_pairs=0, invalid_published_pairs=0,
        single_domain_rows=0, excess_single_domain_admissions=0,
    )
    for article, row in zip(articles, scores, strict=True):
        actual = article.labels
        selected = set(select_domains(row, gate))
        for name in DOMAINS:
            if name in selected:
                per_domain[name]["tp" if name in actual else "fp"] += 1
            elif name in actual:
                per_domain[name]["fn"] += 1
        if not actual:
            out_of_scope += 1
            false_out_of_scope += bool(selected)
        if len(actual) == 1:
            top_two["single_domain_rows"] += 1
            top_two["excess_single_domain_admissions"] += len(selected) == 2
        if len(actual) == 2:
            top_two["pair_rows"] += 1
            top_two["exact_pair_rows"] += selected == actual
        if len(actual) == 3:
            top_two["triad_rows"] += 1
            top_two["valid_triad_pairs"] += len(selected) == 2 and selected.issubset(actual)
        if len(selected) == 2:
            top_two["published_pairs"] += 1
            top_two["invalid_published_pairs"] += not selected.issubset(actual)
    totals = Counter()
    per_domain_metrics = {}
    for name, counts in per_domain.items():
        totals.update(counts)
        per_domain_metrics[name] = {**counts, "f1": f1(**counts)}
    return {
        "per_domain": per_domain_metrics,
        "macro_f1": sum(value["f1"] for value in per_domain_metrics.values()) / len(DOMAINS),
        "micro_f1": f1(**totals),
        "out_of_scope_rows": out_of_scope,
        "false_out_of_scope_admissions": false_out_of_scope,
        "top_two": dict(top_two),
    }


def quality(report):
    return (
        report["macro_f1"],
        -report["false_out_of_scope_admissions"],
        report["micro_f1"],
    )


def promotion_passes(report):
    macro_f1 = report["macro_f1"]
    return type(macro_f1) in (int, float) and MIN_PROMOTION_MACRO_F1 < macro_f1 <= 1.0


def parse_worker_scores(output, batch_sizes, protocol):
    lines = output.splitlines()
    if len(lines) != len(batch_sizes) + 1 or json.loads(lines[0]) != {"protocol": protocol}:
        raise VerifyError("Unexpected production worker handshake or response count")
    scores = []
    for line, size in zip(lines[1:], batch_sizes, strict=True):
        response = json.loads(line)
        if set(response) != {"scores"} or len(response["scores"]) != size:
            raise VerifyError("Unexpected production worker response shape")
        for row in response["scores"]:
            if len(row) != len(DOMAINS) or any(
                type(value) not in (int, float) or not 0 <= value <= 1 for value in row
            ):
                raise VerifyError("Invalid production worker domain score")
            scores.append(row)
    return scores


def score_with_worker(worker, model, articles, contract):
    batch = contract["max_batch"]
    texts = [[item.text for item in articles[at:at + batch]] for at in range(0, len(articles), batch)]
    body = "".join(json.dumps({"texts": chunk}) + "\n" for chunk in texts).encode()
    env = {
        key: os.environ[key]
        for key in ("PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH")
        if key in os.environ
    }
    with tempfile.TemporaryDirectory(prefix="atlas-bert-evaluate-") as temporary:
        output = Commands(Path(temporary)).run(
            "inference", [worker, "serve", model], env=env, body=body, timeout=360, limit=256 * 1024,
        )
    return parse_worker_scores(output, [len(chunk) for chunk in texts], contract["protocol"])


def select_threshold(calibration, baseline_scores, candidate_scores):
    baseline = metrics(calibration, baseline_scores, BASELINE_THRESHOLD)
    best_overall = best_qualified = None
    qualifying_thresholds = 0
    for integer in THRESHOLD_GRID:
        gate = integer / 100
        report = metrics(calibration, candidate_scores, gate)
        key = (*quality(report), gate)
        trial = (key, {"threshold": gate, "metrics": report})
        if best_overall is None or key > best_overall[0]:
            best_overall = trial
        if promotion_passes(report):
            qualifying_thresholds += 1
            if best_qualified is None or key > best_qualified[0]:
                best_qualified = trial
    return {
        "schema_version": CALIBRATION_SCHEMA,
        "status": "qualified" if best_qualified else "rejected",
        "regression_policy": REGRESSION_POLICY,
        "calibration_rows": len(calibration),
        "baseline_threshold": BASELINE_THRESHOLD,
        "baseline": baseline,
        "tested_thresholds": len(THRESHOLD_GRID),
        "qualifying_thresholds": qualifying_thresholds,
        "best_overall": best_overall[1],
        "selected": best_qualified[1] if best_qualified else None,
    }


def calibration_receipt_path(candidate):
    return candidate.with_name(f"{candidate.name}-calibration.json")


def validate_prepared(prepared):
    model = prepared / "model"
    if any(sha(model / file) != digest for file, digest in checkpoint_pins().items()):
        raise VerifyError("Prepared raw model checksum differs from the pinned MNLI checkpoint")
    converter = prepared / "converter"
    cargo = ROOT / "tools/atlas-bert"
    package = next(
        item for item in tomllib.loads((cargo / "Cargo.lock").read_text())["package"]
        if item["name"] == "rust-bert" and item["version"] == "0.22.0"
    )
    archive = converter / "rust-bert.crate"
    if sha(archive) != package["checksum"]:
        raise VerifyError("Prepared converter archive differs from the locked Rust-BERT crate")
    with tarfile.open(archive, "r:gz") as source:
        for name, origin in {
            "utils/convert_model.py": "utils/convert_model.py",
            "src/bin/convert-tensor.rs": "src/convert-tensor.rs",
        }.items():
            with source.extractfile("rust-bert-0.22.0/" + origin) as expected:
                if (converter / name).read_bytes() != expected.read():
                    raise VerifyError("Prepared converter sources differ from the locked crate")
    for name in ("Cargo.toml", "Cargo.lock"):
        if sha(cargo / name) != sha(converter / name):
            raise VerifyError("Prepared converter Cargo inputs differ from the worker")
    return model, converter


def train_checkpoint(checkpoint, train, development, contract, output):
    # These are confined to the offline fitting environment, never the runtime.
    import numpy
    import torch
    import transformers

    if (
        torch.__version__.split("+", 1)[0] != TORCH_VERSION
        or torch.version.cuda is not None
        or transformers.__version__ != TRANSFORMERS_VERSION
        or numpy.__version__ != "1.26.4"
    ):
        raise VerifyError("Fine-tuning requires pinned CPU-only PyTorch, NumPy and Transformers")
    torch.set_num_threads(2)
    torch.set_num_interop_threads(1)
    torch.manual_seed(SEED)
    torch.use_deterministic_algorithms(True)
    tokenizer = transformers.DistilBertTokenizer(
        vocab_file=str(checkpoint / "vocab.txt"), do_lower_case=True,
    )
    config = transformers.DistilBertConfig.from_json_file(checkpoint / "config.json")
    labels = {label.lower(): index for index, label in config.id2label.items()}
    if labels != {"entailment": 0, "neutral": 1, "contradiction": 2}:
        raise VerifyError("Fine-tuning requires the pinned MNLI label ordering")
    model = transformers.DistilBertForSequenceClassification(config)
    weights = torch.load(checkpoint / "pytorch_model.bin", map_location="cpu", weights_only=True)
    positions = weights.pop("distilbert.embeddings.position_ids", None)
    if positions is not None and not torch.equal(positions, model.distilbert.embeddings.position_ids):
        raise VerifyError("Pinned checkpoint position IDs differ from DistilBERT")
    model.load_state_dict(weights, strict=True)
    del weights, positions

    train_layers = (f"distilbert.transformer.layer.{config.n_layers - 1}.", "pre_classifier.", "classifier.")
    for name, parameter in model.named_parameters():
        parameter.requires_grad_(name.startswith(train_layers))
    optimizer = torch.optim.AdamW((param for param in model.parameters() if param.requires_grad),
                                  lr=LEARNING_RATE, weight_decay=.01)
    hypotheses = contract["hypotheses"]

    def encode(articles):
        pairs = [(article, domain, hypothesis) for article in articles
                 for domain, hypothesis in zip(DOMAINS, hypotheses, strict=True)]
        encoded = tokenizer(
            [article.text for article, _, _ in pairs], [hypothesis for _, _, hypothesis in pairs],
            truncation="longest_first", max_length=contract["max_tokens"],
            return_token_type_ids=False, return_attention_mask=False,
        )["input_ids"]
        targets = [0 if domain in article.labels else 1 for article, domain, _ in pairs]
        return encoded, targets

    def batches(encoded, order):
        for start in range(0, len(order), TRAIN_BATCH):
            indices = order[start:start + TRAIN_BATCH]
            batch = tokenizer.pad({"input_ids": [encoded[index] for index in indices]}, return_tensors="pt")
            yield indices, batch

    train_encoded, train_targets = encode(train)
    development_encoded, _ = encode(development)
    generator = torch.Generator().manual_seed(SEED)
    best = None
    comparisons = []
    for epoch in range(1, EPOCHS + 1):
        model.train()
        order = torch.randperm(len(train_encoded), generator=generator).tolist()
        for indices, batch in batches(train_encoded, order):
            optimizer.zero_grad(set_to_none=True)
            logits = model(**batch).logits[:, [labels["entailment"], labels["contradiction"]]]
            loss = torch.nn.functional.cross_entropy(logits, torch.tensor([train_targets[index] for index in indices]))
            if not torch.isfinite(loss).item():
                raise VerifyError("Fine-tuning produced a non-finite training loss")
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0, error_if_nonfinite=True)
            optimizer.step()
        model.eval()
        scores = []
        with torch.inference_mode():
            for _, batch in batches(development_encoded, list(range(len(development_encoded)))):
                logits = model(**batch).logits[:, [labels["entailment"], labels["contradiction"]]]
                scores.extend(logits.softmax(dim=-1)[:, 0].tolist())
        if any(not 0 <= score <= 1 for score in scores):
            raise VerifyError("Fine-tuning produced an invalid development score")
        report = metrics(development, [scores[i:i + len(DOMAINS)] for i in range(0, len(scores), len(DOMAINS))],
                         BASELINE_THRESHOLD)
        comparisons.append({"epoch": epoch, **report})
        key = quality(report)
        if best is None or key > best[0]:
            best = (key, epoch)
            torch.save(model.state_dict(), output)
        print(f"Atlas BERT development epoch {epoch}/{EPOCHS} complete", flush=True)
    return {
        "format": "atlas-bert-nli-final-block-v1",
        "seed": SEED, "selected_epoch": best[1], "epochs": EPOCHS, "batch_pairs": TRAIN_BATCH,
        "learning_rate": LEARNING_RATE, "torch": torch.__version__, "numpy": numpy.__version__,
        "transformers": transformers.__version__, "python": platform.python_version(), "threads": 2,
        "train_layers": list(train_layers), "development": comparisons,
    }


def write_new(path, value):
    # Readers never observe a half-written freeze or a replaced evaluation.
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, encoding="utf-8") as output:
        output.write(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n")
        output.flush()
        os.fsync(output.fileno())
        os.link(output.name, path)


def fit(prepared, destination, worker, revision):
    if destination.exists() or destination.is_symlink():
        raise VerifyError("Choose a new candidate directory")
    receipt_path = calibration_receipt_path(destination)
    if receipt_path.exists() or receipt_path.is_symlink():
        raise VerifyError("Candidate already has a calibration receipt")
    checkpoint, converter = validate_prepared(prepared)
    runtime = runtime_contract()
    articles = read_articles(read_corpus(), ("train", "development", "calibration"))
    with tempfile.TemporaryDirectory(prefix=".atlas-bert-fit-", dir=destination.parent) as temporary:
        staging = Path(temporary) / "model"
        staging.mkdir(mode=0o700)
        weights = staging / "pytorch_model.bin"
        training = train_checkpoint(checkpoint, articles["train"], articles["development"], runtime, weights)
        for name in ("config.json", "vocab.txt"):
            shutil.copyfile(checkpoint / name, staging / name)
        preparer = ROOT / "tools/atlas-bert/scripts/prepare-model.sh"
        conversion_env = os.environ.copy()
        conversion_env["PATH"] = f"{Path(sys.executable).parent}{os.pathsep}{os.environ.get('PATH', os.defpath)}"
        subprocess.run(
            ["bash", preparer, "--convert", weights, converter], check=True, timeout=300, env=conversion_env,
        )
        weights.unlink()
        calibration = articles["calibration"]
        baseline_scores = score_with_worker(worker, checkpoint, calibration, runtime)
        candidate_scores = score_with_worker(worker, staging, calibration, runtime)
        qualification = select_threshold(calibration, baseline_scores, candidate_scores)
        candidate_assets = model_assets(staging)
        baseline_assets = model_assets(checkpoint)
        worker_sha = sha(worker)
        qualification.update({
            "dataset_sha256": DATASET_SHA256,
            "source_revision": revision,
            "worker_sha256": worker_sha,
            "baseline_assets": baseline_assets,
            "candidate_assets": candidate_assets,
        })
        write_new(receipt_path, qualification)
        selected = qualification["selected"]
        if selected is None:
            return None
        frozen = {
            "schema_version": "atlas-bert-frozen-v1", "dataset_sha256": DATASET_SHA256,
            "base_checkpoint_revision": MODEL_COMMIT, "split_counts": SPLIT_COUNTS,
            "threshold": selected["threshold"], "assets": candidate_assets,
            "provenance": {
                "kind": "synthetic", "training": training, "calibration": selected["metrics"],
                "calibration_qualification": qualification,
                "runtime_contract": runtime, "source_revision": revision,
                "trainer_sha256": sha(Path(__file__).resolve()),
                "converter_source_sha256": sha(converter / "rust-bert.crate"),
                "conversion_script_sha256": sha(preparer),
                "calibration_worker_sha256": worker_sha,
                "baseline_assets": baseline_assets,
            },
        }
        write_new(staging / FROZEN_NAME, frozen)
        # Staging is on the same filesystem. Never install a partial model.
        if destination.exists() or destination.is_symlink():
            raise VerifyError("Candidate destination was created concurrently")
        staging.rename(destination)
    return frozen


def load_frozen(candidate, baseline, worker=None):
    with (candidate / FROZEN_NAME).open("rb") as source:
        frozen_bytes = source.read(64 * 1024 + 1)
    if len(frozen_bytes) > 64 * 1024:
        raise VerifyError("Frozen manifest exceeds its size bound")
    frozen = json.loads(frozen_bytes)
    if (
        frozen["schema_version"] != "atlas-bert-frozen-v1"
        or frozen["dataset_sha256"] != DATASET_SHA256
        or frozen["split_counts"] != SPLIT_COUNTS
        or frozen["base_checkpoint_revision"] != MODEL_COMMIT
        or frozen["assets"] != model_assets(candidate)
        or frozen["provenance"]["baseline_assets"] != model_assets(baseline)
        or frozen["provenance"]["runtime_contract"] != runtime_contract()
        or (worker is not None and frozen["provenance"]["calibration_worker_sha256"] != sha(worker))
    ):
        raise VerifyError("Frozen model, baseline, worker, corpus or runtime contract differs")
    threshold(frozen["threshold"])
    provenance = frozen["provenance"]
    if "calibration_qualification" in provenance:
        qualification = provenance["calibration_qualification"]
        selected = qualification.get("selected") or {}
        if (
            qualification["schema_version"] != CALIBRATION_SCHEMA
            or qualification["status"] != "qualified"
            or qualification["regression_policy"] != REGRESSION_POLICY
            or qualification["calibration_rows"] != SPLIT_COUNTS["calibration"]
            or qualification["baseline_threshold"] != BASELINE_THRESHOLD
            or qualification["tested_thresholds"] != len(THRESHOLD_GRID)
            or not 1 <= qualification["qualifying_thresholds"] <= len(THRESHOLD_GRID)
            or qualification["dataset_sha256"] != frozen["dataset_sha256"]
            or qualification["source_revision"] != provenance["source_revision"]
            or qualification["worker_sha256"] != provenance["calibration_worker_sha256"]
            or qualification["baseline_assets"] != provenance["baseline_assets"]
            or qualification["candidate_assets"] != frozen["assets"]
            or selected.get("threshold") != frozen["threshold"]
            or selected.get("metrics") != provenance["calibration"]
            or not promotion_passes(selected["metrics"])
        ):
            raise VerifyError("Frozen candidate calibration qualification differs")
    return frozen, hashlib.sha256(frozen_bytes).hexdigest()


def evaluate(candidate, baseline, worker):
    if (candidate / EVALUATION_NAME).exists():
        raise VerifyError("Candidate already has an immutable held-out evaluation")
    frozen, frozen_hash = load_frozen(candidate, baseline, worker)
    if "calibration_qualification" not in frozen["provenance"]:
        raise VerifyError("A current calibration qualification is required before held-out scoring")
    # Held-out labels and articles are first materialized after freeze verification.
    articles = read_articles(read_corpus(), ("test",))["test"]
    runtime = frozen["provenance"]["runtime_contract"]
    started = time.perf_counter()
    baseline_scores = score_with_worker(worker, baseline, articles, runtime)
    baseline_ms = (time.perf_counter() - started) * 1000
    started = time.perf_counter()
    candidate_scores = score_with_worker(worker, candidate, articles, runtime)
    candidate_ms = (time.perf_counter() - started) * 1000
    baseline_metrics = metrics(articles, baseline_scores, BASELINE_THRESHOLD)
    candidate_metrics = metrics(articles, candidate_scores, frozen["threshold"])
    if load_frozen(candidate, baseline, worker)[1] != frozen_hash:
        raise VerifyError("Frozen model changed while evaluating")
    results = {
        "schema_version": "atlas-bert-evaluation-v1", "frozen_sha256": frozen_hash,
        "dataset_sha256": DATASET_SHA256, "held_out_rows": len(articles),
        "worker_sha256": sha(worker), "baseline_threshold": BASELINE_THRESHOLD,
        "baseline_base_checkpoint_revision": MODEL_COMMIT, "baseline_assets": model_assets(baseline),
        "regression_policy": REGRESSION_POLICY,
        "baseline": baseline_metrics, "candidate": candidate_metrics,
        "worker_wall_ms": {"baseline": baseline_ms, "candidate": candidate_ms,
                           "architecture": platform.machine(), "samples_per_model": 1,
                           "includes_startup": True},
        "passed": promotion_passes(candidate_metrics),
    }
    write_new(candidate / EVALUATION_NAME, results)
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    fitting = commands.add_parser("fit")
    fitting.add_argument("--prepared", type=Path, required=True)
    fitting.add_argument("--revision", required=True)
    held_out = commands.add_parser("evaluate")
    held_out.add_argument("--baseline", type=Path, required=True)
    for command in (fitting, held_out):
        command.add_argument("--candidate", type=Path, required=True)
        command.add_argument("--worker", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        worker = args.worker.resolve(strict=True)
        candidate = args.candidate.absolute()
        if args.command == "fit":
            if not re.fullmatch(r"[a-f0-9]{40}", args.revision):
                raise VerifyError("An exact source SHA is required")
            revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
            if args.revision != revision:
                raise VerifyError("Checked-out trainer source differs from the requested SHA")
            result = fit(args.prepared.resolve(strict=True), candidate, worker, args.revision)
            if result is None:
                print("Atlas BERT candidate did not exceed 0.65 macro F1 on calibration", file=sys.stderr)
                return 1
            print(json.dumps({"threshold": result["threshold"], "assets": result["assets"]}))
        else:
            result = evaluate(candidate, args.baseline.resolve(strict=True), worker)
            print(json.dumps(result))
            if not result["passed"]:
                return 1
    except (VerifyError, OSError, ValueError, KeyError, RuntimeError, TypeError, ImportError, subprocess.SubprocessError) as error:
        print("Atlas BERT candidate step failed:", error, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
