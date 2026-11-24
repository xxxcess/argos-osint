#!/usr/bin/env python3
"""Compare one pinned Inbox reply candidate on an isolated GitHub Actions runner."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import resource
import statistics
import subprocess
import tempfile
import time
import urllib.request


SENTENCEPIECE = (791656, "d60acb128cf7b7f2536e8f38a5b18a05535c9e14c7a355904270e15b0945ea86")
GPT2_TOKENIZER = {
    "merges.txt": (456318, "1ce1664773c50f3e0cc8842619a93edc4624525b728b188a9e0be33b7726adc5"),
    "vocab.json": (1042301, "196139668be63f3b5d6574427317ae82f612a97c5d1cdaf36ed2256dbf636783"),
}
CANDIDATES = {
    "flan-t5-small": (
        "t5",
        "google/flan-t5-small",
        "0fc9ddf78a1e988dac52e2dac162b0ede4fd74ab",
        {
            "config.json": (1401, "439aa0fecf5a5546a1def68b1fc45e538e2c94528ce805378daf091e2bf6e4de"),
            "model.safetensors": (307867048, "495fa51e204676f1a857a9fc13c4c89f3f5ba9f480b898cebca02add25e6d749"),
            "spiece.model": SENTENCEPIECE,
        },
        {
            "d_model": 512, "d_ff": 1024, "num_layers": 8, "num_decoder_layers": 8,
            "num_heads": 6, "vocab_size": 32128, "feed_forward_proj": "gated-gelu",
            "tie_word_embeddings": False,
        },
    ),
    "flan-t5-base": (
        "t5",
        "google/flan-t5-base",
        "7bcac572ce56db69c1ea7c8af255c5d7c9672fc2",
        {
            "config.json": (1404, "7c1853dbfa0e4aac093eb109a358b6ab25fe86b7c15185a91322f0ed26f0f940"),
            "model.safetensors": (990345061, "1dfb70afdcedceb9f9fae2f9b68e004ad934361fb35b9b2bd50b45ea90790fc8"),
            "spiece.model": SENTENCEPIECE,
        },
        {
            "d_model": 768, "d_ff": 2048, "num_layers": 12, "num_decoder_layers": 12,
            "num_heads": 12, "vocab_size": 32128, "feed_forward_proj": "gated-gelu",
            "tie_word_embeddings": False,
        },
    ),
    "gpt2-small": (
        "gpt2",
        "openai-community/gpt2",
        "607a30d783dfa663caf39e06633721c8d4cfcd7e",
        {
            "config.json": (665, "0daed7749b4f02b8f76240d5444551d7b08712dab4d0adb8239c56ba823bb7b4"),
            "rust_model.ot": (702517648, "adf0adedbf4016b249550f866c66a3b3a3d09c8b3b3a1f6e5e9a265d94e0270e"),
            **GPT2_TOKENIZER,
        },
        {"n_embd": 768, "n_head": 12, "n_layer": 12, "vocab_size": 50257},
    ),
    "gpt2-medium": (
        "gpt2",
        "openai-community/gpt2-medium",
        "6dcaa7a952f72f9298047fd5137cd6e4f05f41da",
        {
            "config.json": (718, "ef1a44d889ad1a0acc7731c78134f1b87d2d222f110e97dd10fd4117331caf22"),
            "rust_model.ot": (1725881760, "064e9fde8e3a539c41b186a6ca94e6fb7c6520f49f903fb236f6e89912fedd32"),
            **GPT2_TOKENIZER,
        },
        {"n_embd": 1024, "n_head": 16, "n_layer": 24, "vocab_size": 50257},
    ),
}

# Synthetic, public observations. Each tuple gives the category, supplied text,
# question, required phrase groups, exact numbers, and forbidden phrases.
CASES = {
    "readiness": (
        "operational", "The Harbor relay is degraded. The Cedar gateway is ready.",
        "Which component is degraded?", (("harbor relay",),), (), ("cedar gateway",),
    ),
    "queue-depth": (
        "operational", "Seven reports are waiting. The classifier has 18 queued jobs and 3 running jobs.",
        "How many classifier jobs are queued?", (), ("18",), ("running",),
    ),
    "memory-sample": (
        "operational", "Node Juniper uses 5.25 GiB of memory. Its disk uses 32 GiB.",
        "How much memory is Juniper using?", (("gib",),), ("5.25",), ("disk",),
    ),
    "worker-count": (
        "operational", "The parser restarted twice. There are 14 active workers and 11 idle workers.",
        "How many workers are active?", (), ("14",), ("idle",),
    ),
    "two-observations": (
        "operational", "The Atlas importer reports 9 failed jobs. The Orchid importer reports 2 failed jobs.",
        "Give the failed job counts for Atlas and Orchid.",
        (("atlas",), ("orchid",)), ("9", "2"), (),
    ),
    "current-reading": (
        "operational", "At noon the intake rate was 12 entries per minute. The current rate is 24 entries per minute.",
        "What is the current intake rate?",
        (("per minute", "entries a minute", "entries/min"),), ("24",), (),
    ),
    "exact-status": (
        "operational", "The classifier is paused. The indexer is stopped. The gateway is ready.",
        "What is the status of the classifier?", (("paused",),), (), ("stopped", "ready"),
    ),
    "history-first-exchange": (
        "recall", "Earlier user: Call this workspace Red Kite.\nEarlier assistant: I will call it Red Kite.\n"
        "Earlier user: How many workers were recorded?\nEarlier assistant: The sample listed 6 workers.",
        "What name did I give this workspace?", (("red kite",),), (), ("workers",),
    ),
    "history-second-exchange": (
        "recall", "Earlier user: Which queue is degraded?\nEarlier assistant: The Amber export queue is degraded.\n"
        "Earlier user: Call the follow-up Ice Fern.\nEarlier assistant: I will call the follow-up Ice Fern.",
        "What name did I give the follow-up?", (("ice fern",),), (), ("amber",),
    ),
    "unknown-reading": (
        "missing-evidence", "Lark's kit reports CPU usage of 19 percent. No disk sample is present.",
        "How much disk space is Lark using?",
        (("unknown", "not available", "not present", "not provided", "not specified", "no disk sample",
          "do not know", "don't know", "cannot determine", "can't determine", "does not say"),),
        (), ("19",),
    ),
    "greeting": (
        "conversation", "", "Hello!",
        (("hello", "hi", "welcome", "greetings"),), (), ("degraded", "ready", "failed"),
    ),
    "acknowledgement": (
        "conversation", "", "Thanks for your help.",
        (("welcome", "glad", "happy", "anytime", "pleasure", "no problem"),),
        (), ("degraded", "ready", "failed"),
    ),
}

INSTRUCTIONS = {
    "operational": "Answer the question using only the approved observations. Keep names, statuses and numbers unchanged.",
    "recall": "Answer the question using only the earlier conversation.",
    "missing-evidence": "Answer using only the approved observations. Say you do not know if the answer is unavailable.",
    "conversation": "Reply to the operator in one friendly sentence without making claims about workstation conditions.",
}
DEFAULT_EMBEDDED_CAPS_MIB = {
    "flan-t5-small": 1536,
    "flan-t5-base": 3584,
    "gpt2-small": 2048,
    "gpt2-medium": 4608,
}
NUMBER = re.compile(r"(?<![\w.])[-+]?\d+(?:[.,]\d+)*(?!\w)")


def prompt(kind, text, question):
    context = f"Approved observations:\n{text}\n" if kind in {"operational", "missing-evidence"} else ""
    if kind == "recall":
        context = f"{text}\n"
    return f"{INSTRUCTIONS[kind]} Keep your reply brief.\n{context}Current user: {question}\nReply:"


def contains_phrase(answer, phrase):
    return re.search(rf"(?<!\w){re.escape(phrase)}(?!\w)", answer) is not None


def score(case_id, answer):
    kind, _, _, required, expected_numbers, forbidden = CASES[case_id]
    text = " ".join(answer.casefold().split())
    found_numbers = set(NUMBER.findall(text))
    missing = [phrases for phrases in required if not any(contains_phrase(text, part) for part in phrases)]
    unexpected = [phrase for phrase in forbidden if contains_phrase(text, phrase)]
    linked = True
    if case_id == "two-observations":
        references = re.findall(r"\b(?:atlas|orchid|9|2)\b", text)
        linked = references in (
            ["atlas", "9", "orchid", "2"], ["orchid", "2", "atlas", "9"],
            ["9", "atlas", "2", "orchid"], ["2", "orchid", "9", "atlas"],
        ) or (
            contains_phrase(text, "respectively")
            and references in (["atlas", "orchid", "9", "2"], ["orchid", "atlas", "2", "9"])
        )
    return {
        "category": kind,
        "passed": bool(text) and not missing and not unexpected and linked and found_numbers == set(expected_numbers),
        "required_phrase_groups": required,
        "missing_phrase_groups": missing,
        "forbidden_phrases_found": unexpected,
        "expected_numbers": expected_numbers,
        "observed_numbers": sorted(found_numbers),
        "fact_bindings_correct": linked,
    }


def fetch(source, revision, filename, expected_bytes, expected_digest, directory):
    url = f"https://huggingface.co/{source}/resolve/{revision}/{filename}"
    for attempt in range(3):
        digest = hashlib.sha256()
        received = 0
        try:
            with urllib.request.urlopen(url, timeout=70) as stream, (directory / filename).open("wb") as output:
                while block := stream.read(1024 * 1024):
                    received += len(block)
                    if received > expected_bytes:
                        raise ValueError(f"{filename} exceeded its pinned size")
                    digest.update(block)
                    output.write(block)
            if received != expected_bytes or digest.hexdigest() != expected_digest:
                raise ValueError(f"{filename} did not match its pinned size and SHA-256")
            return
        except (OSError, ValueError):
            if attempt == 2:
                raise
            time.sleep(2 ** attempt)


def evaluate(args, receipt, model):
    family, source, revision, files, shape = CANDIDATES[args.candidate]
    receipt["status"] = "asset_verification_failed"
    verified = []
    for filename, (size, digest) in files.items():
        fetch(source, revision, filename, size, digest, model)
        verified.append({"file": filename, "bytes": size, "sha256": digest})
    config = json.loads((model / "config.json").read_text())
    if any(config.get(key) != value for key, value in shape.items()):
        raise ValueError("checkpoint config does not match its pinned architecture")
    receipt.update(verified_files=verified, architecture=shape)

    prompts = [{"id": case_id, "text": prompt(*case[:3])} for case_id, case in CASES.items()]
    child_env = dict(os.environ, OMP_NUM_THREADS="1", MKL_NUM_THREADS="1", RAYON_NUM_THREADS="1")
    receipt["status"] = "worker_failed"
    started = time.monotonic()
    try:
        completed = subprocess.run(
            [args.worker, family, model], input=json.dumps(prompts), text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=child_env, timeout=300, check=False,
        )
    finally:
        # This driver spawns only the evaluator. Linux reports ru_maxrss in KiB.
        peak_bytes = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * 1024
        receipt.update(
            wall_ms=round((time.monotonic() - started) * 1000),
            peak_rss_bytes=peak_bytes,
            worker_rss_within_default_embedded_cap=0 < peak_bytes <= receipt["default_embedded_cap_bytes"],
        )
    if completed.returncode != 0:
        if completed.returncode > 0 and (
            completed.stderr.startswith("Error: load ") and not completed.stdout
        ):
            receipt["status"] = "incompatible_checkpoint"
        raise ValueError(f"worker exit {completed.returncode}\n{completed.stderr[-4000:]}")

    events = [json.loads(line) for line in completed.stdout.splitlines()]
    if len(events) != len(CASES) + 1 or [event.get("id") for event in events[1:]] != list(CASES):
        raise ValueError("worker did not return one result per fixture in order")
    cases = [dict(event, **score(event["id"], event["answer"])) for event in events[1:]]
    categories = {}
    for case in cases:
        counts = categories.setdefault(case["category"], {"passed": 0, "total": 0})
        counts["passed"] += case["passed"]
        counts["total"] += 1
    receipt.update(
        status="completed",
        model_load_ms=events[0]["model_load_ms"],
        median_case_ms=statistics.median(case["latency_ms"] for case in cases),
        max_case_ms=max(case["latency_ms"] for case in cases),
        first_case_ms=cases[0]["latency_ms"],
        passed=sum(case["passed"] for case in cases),
        total=len(cases),
        categories=categories,
        cases=cases,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True, choices=CANDIDATES)
    parser.add_argument("--head", required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        parser.error("the evaluator requires an AMD64 Linux runner")
    if not re.fullmatch(r"[0-9a-f]{40}", args.head):
        parser.error("an exact source SHA is required")

    _, source, revision, _, _ = CANDIDATES[args.candidate]
    receipt = {
        "schema": 1,
        "source_sha": args.head,
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
        "candidate": args.candidate,
        "checkpoint": source,
        "checkpoint_revision": revision,
        "target": "x86_64-unknown-linux-gnu",
        "runtime": "rust-bert 0.22.0 CPU",
        "default_embedded_cap_bytes": DEFAULT_EMBEDDED_CAPS_MIB[args.candidate] * 1024 ** 2,
        "measurement_scope": "single generator process; excludes retrieval model and packaged app",
        "input_token_limit": 256,
        "output_token_limit": 48,
    }
    try:
        with tempfile.TemporaryDirectory(prefix="inbox-qa-evaluation-") as model:
            evaluate(args, receipt, Path(model))
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        receipt["diagnostics"] = str(error)[-4200:]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    print(f"{args.candidate}: {receipt['status']}")
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a") as output:
            print(f"### Inbox QA candidate `{args.candidate}`", file=output)
            print(f"\nCheckpoint `{source}@{revision}` on source `{args.head}`.\n", file=output)
            if receipt["status"] == "completed":
                print(f"Fixed fixtures: {receipt['passed']}/{receipt['total']} passed. "
                      f"Median {receipt['median_case_ms']} ms, maximum {receipt['max_case_ms']} ms.\n", file=output)
            else:
                print(f"Status: `{receipt['status']}`. See the receipt for diagnostics.\n", file=output)
            if "peak_rss_bytes" in receipt:
                print(f"Generator process peak: {receipt['peak_rss_bytes'] / 1024**2:.1f} MiB. "
                      f"Default Embedded cap: {DEFAULT_EMBEDDED_CAPS_MIB[args.candidate]} MiB. "
                      "Retrieval and app memory are excluded.", file=output)
    return int(receipt["status"] != "completed")


if __name__ == "__main__":
    raise SystemExit(main())
