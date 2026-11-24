#!/usr/bin/env python3
"""Measure one pinned, text-only System One candidate over a private socket."""

import argparse
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import platform
import signal
import socket
import statistics
import subprocess
import tarfile
import tempfile
import time
from dataclasses import dataclass

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
PINS = json.loads((FIXTURES / "fleetworks-decision-pins.json").read_text())
INCIDENTS = FIXTURES / "fleetworks-decision-incidents.json"
TARGETS = {("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
           ("Darwin", "x86_64"): "x86_64-apple-darwin",
           ("Darwin", "arm64"): "aarch64-apple-darwin"}
NONE = "none"
MAX_BODY = 8192
EIGHT_CANDIDATE_TRIALS = 8


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "Duplicate JSON field in decision response")
        result[key] = value
    return result


def probability(value):
    return type(value) in (int, float) and 0 <= value <= 1


class UnixConnection(http.client.HTTPConnection):
    def __init__(self, path):
        super().__init__("localhost", timeout=5)
        self.path = path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.path)


def post(path, method, url, payload=None):
    connection = UnixConnection(path)
    try:
        body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
        require(body is None or len(body) <= MAX_BODY, "Decision request exceeds its byte limit")
        connection.request(method, url, body, {"Content-Type": "application/json"})
        with connection.getresponse() as response:
            if response.status != 200:
                raise ValueError(f"Decision service returned HTTP {response.status}")
            data = response.read(MAX_BODY + 1)
        require(0 < len(data) <= MAX_BODY, "Decision response exceeds its byte limit")
        return json.loads(data, object_pairs_hook=unique_object)
    finally:
        connection.close()


@dataclass(frozen=True)
class Decision:
    choice: str
    probability: float
    margin: float


def decode_choice(raw, model, handles):
    require(type(raw) is dict and set(raw) == {"model", "answers", "usage"} and raw["model"] == model,
            "Unexpected decision response identity or fields")
    usage = raw["usage"]
    require(type(usage) is dict and set(usage) == {"input_tokens", "output_tokens"}
            and type(usage["input_tokens"]) is int and 0 < usage["input_tokens"] <= 16 * 1024
            and type(usage["output_tokens"]) is int and usage["output_tokens"] == 0,
            "Unexpected decision token accounting")
    answers = raw["answers"]
    require(type(answers) is dict and set(answers) == {"related"}, "Unexpected decision question ID")
    answer = answers["related"]
    require(type(answer) is dict and set(answer) == {"type", "choice", "probabilities", "confidence"}
            and answer["type"] == "choice" and probability(answer["confidence"]),
            "Unexpected decision answer shape")
    weights = answer["probabilities"]
    require(type(weights) is dict and set(weights) == handles and all(map(probability, weights.values()))
            and abs(math.fsum(weights.values()) - 1) <= 0.0001,
            "Model did not return a normalized distribution over the exact menu")
    choice = answer["choice"]
    require(type(choice) is str and choice in handles and weights[choice] == max(weights.values()),
            "Model chose a handle outside its menu or disagreed with its distribution")
    runner_up = max(value for handle, value in weights.items() if handle != choice)
    return Decision(choice, weights[choice], weights[choice] - runner_up)


def case_data():
    cases = json.loads(INCIDENTS.read_text())
    require(set(cases) == {"calibration", "held_out"}, "Invalid incident groups")
    seen = set()
    for group in cases.values():
        require(1 <= len(group) <= 64, "Invalid incident group size")
        for case in group:
            require(len(case) == 4 or (len(case) == 5 and case[4] == "direct"), "Invalid incident")
            incident, state, candidates, expected, *_ = case
            require(incident not in seen and 1 <= len(state.encode()) <= 256 and 2 <= len(candidates) <= 8,
                    "Duplicate or unbounded incident")
            seen.add(incident)
            require(expected is None or type(expected) is int and 0 <= expected < len(candidates),
                    "Expected handle not present")
            require(len({description for _, description in candidates}) == len(candidates)
                    and all(type(rank) is int and 0 <= rank <= 8 and 1 <= len(description.encode()) <= 192
                            for rank, description in candidates), "Invalid candidate menu")
    return cases


def choose(path, model, state, candidates):
    menu = {f"c{index}": description for index, (_, description) in enumerate(candidates)}
    menu[NONE] = "No listed observation establishes a supported relationship to the selected subject."
    response = post(path, "POST", "/v1/systemone", {
        "state": state,
        "questions": {"related": {"type": "choice", "criteria": menu,
                                  "instructions": "Choose one supported relationship to the selected subject, or none."}},
    })
    return decode_choice(response, model, set(menu))


def expected_choice(case):
    return NONE if case[3] is None else f"c{case[3]}"


def counts(cases, choices):
    answerable = sum(case[3] is not None for case in cases)
    displayed = sum(choice != NONE for choice in choices)
    on_answerable = sum(choice != NONE and case[3] is not None for case, choice in zip(cases, choices, strict=True))
    correct = sum(choice != NONE and choice == expected_choice(case)
                  for case, choice in zip(cases, choices, strict=True))
    return {"answerable": answerable, "displayed": displayed, "correct": correct,
            "false_suggestions": displayed - correct, "abstentions": len(cases) - displayed,
            "precision": correct / displayed if displayed else 0,
            "coverage": on_answerable / answerable if answerable else 0}


def thresholded(decisions, threshold, margin):
    return [decision.choice if decision.probability >= threshold and decision.margin >= margin
            else NONE for decision in decisions]


def calibrate(cases, decisions):
    qualified = []
    for threshold in (0.4, 0.5, 0.6, 0.7, 0.8, 0.9):
        for margin in (0.05, 0.1, 0.2, 0.3):
            stats = counts(cases, thresholded(decisions, threshold, margin))
            if stats["precision"] >= 0.90:
                qualified.append((stats["coverage"], threshold, margin))
    if not qualified:
        return 1.0, 1.0
    _, threshold, margin = max(qualified)
    return threshold, margin


def baseline(cases, displayed):
    ranked = []
    for position, case in enumerate(cases):
        ranks = sorted(((rank, index) for index, (rank, _) in enumerate(case[2])), reverse=True)
        score, choice = ranks[0]
        margin = score - ranks[1][0]
        if score >= 2 and margin > 0:
            ranked.append((margin, score, position, choice))
    results = [NONE] * len(cases)
    for _, _, position, choice in sorted(ranked, reverse=True)[:displayed]:
        results[position] = f"c{choice}"
    return counts(cases, results)



def fetch_runtime(workspace, target, receipt):
    runtime = PINS["runtime"]
    archive = runtime["archives"][target]
    package = workspace / archive["file"]
    download(f"https://github.com/ggml-org/llama.cpp/releases/download/{runtime['tag']}/{archive['file']}",
             package, archive["bytes"])
    require(package.stat().st_size == archive["bytes"] and sha256(package) == archive["sha256"],
            "Decision runtime differed from its pinned archive")
    directory = workspace / "runtime"
    directory.mkdir(mode=0o700)
    with tarfile.open(package, "r:gz") as bundle:
        entries = bundle.getmembers()
        require(1 <= len(entries) <= 512 and sum(entry.size for entry in entries) <= 256 * 1024 * 1024,
                "Decision runtime archive exceeds its extraction budget")
        bundle.extractall(directory, members=entries, filter="data")
    workers = [path for path in directory.rglob("llama-server") if path.is_file() and not path.is_symlink()]
    require(len(workers) == 1 and os.access(workers[0], os.X_OK),
            "Expected one executable decision server in the pinned archive")
    worker, = workers
    receipt.update(worker_sha256=sha256(worker), worker_bytes=worker.stat().st_size)
    return worker


def fetch(model, workspace):
    base = f"https://huggingface.co/{model['repository']}"
    if "license_sha256" in model:
        license_file = workspace / "LICENSE"
        download(f"{base}/raw/{model['revision']}/LICENSE", license_file, model["license_bytes"])
        require(license_file.stat().st_size == model["license_bytes"]
                and sha256(license_file) == model["license_sha256"], "Model license differed from its pin")
    file = workspace / model["file"]
    download(f"{base}/resolve/{model['revision']}/{model['file']}", file, model["bytes"])
    require(file.stat().st_size == model["bytes"] and sha256(file) == model["sha256"],
            "Checkpoint size or digest differed from its pinned identity")
    return file


def download(url, file, max_bytes):
    subprocess.run(["curl", "--fail", "--silent", "--show-error", "--location",
                    "--proto", "=https", "--proto-redir", "=https", "--max-redirs", "5",
                    "--retry", "2", "--retry-max-time", "600", "--max-time", "600",
                    "--max-filesize", str(max_bytes), url,
                    "--output", str(file)], check=True, timeout=620)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def sample_rss(process):
    if platform.system() == "Linux":
        status = Path(f"/proc/{process.pid}/status").read_text()
        kib = next(int(line.split()[1]) for line in status.splitlines() if line.startswith("VmHWM:"))
    else:
        result = subprocess.run(["/bin/ps", "-o", "rss=", "-p", str(process.pid)],
                                check=True, capture_output=True, timeout=3)
        kib = int(result.stdout.strip())
    return kib * 1024


def ready(path, process):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline and process.poll() is None:
        try:
            if post(path, "GET", "/health") == {"status": "ok"}:
                return
        except (OSError, ValueError):
            time.sleep(0.1)
    raise ValueError("Pinned decision worker did not become healthy")


def qualify(args, receipt, workspace, model):
    worker = fetch_runtime(workspace, args.target, receipt)
    checkpoint = fetch(model, workspace)
    socket_path = str(workspace / "score.sock")
    environment = {"HOME": str(workspace), "PATH": os.defpath, "LANG": "C", "LC_ALL": "C"}
    if platform.system() == "Linux":
        environment["LD_LIBRARY_PATH"] = str(worker.parent)
    command = [str(worker), "--model", str(checkpoint), "--alias", args.model,
               "--host", socket_path, "--offline", "--no-webui", "--no-mmproj", "--no-cache-prompt",
               "--no-slots", "--parallel", "1", "--threads", "1", "--threads-batch", "1",
               "--threads-http", "1", "--ctx-size", "1024", "--n-gpu-layers", "0"]
    cases = case_data()
    started = time.monotonic()
    with subprocess.Popen(command, cwd=workspace, env=environment, start_new_session=True,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
        try:
            ready(socket_path, process)
            receipt["cold_ready_ms"] = round((time.monotonic() - started) * 1000, 2)
            first_started = time.monotonic()
            first_case = cases["calibration"][0]
            choose(socket_path, args.model, first_case[1], first_case[2])
            receipt["first_decision_ms"] = round((time.monotonic() - first_started) * 1000, 2)
            evaluated = {}
            direct_count = 0
            peak = sample_rss(process)
            latencies = []
            for group, group_cases in cases.items():
                ambiguous = []
                scored = []
                for case in group_cases:
                    if len(case) == 5:
                        direct_count += 1
                        continue
                    began = time.monotonic()
                    scored.append(choose(socket_path, args.model, case[1], case[2]))
                    if group == "held_out":
                        latencies.append((time.monotonic() - began) * 1000)
                    peak = max(peak, sample_rss(process))
                    ambiguous.append(case)
                evaluated[group] = (ambiguous, scored)
            eight_menu_case, = (case for case in cases["held_out"] if len(case[2]) == 8)
            eight_state, eight_menu = eight_menu_case[1:3]
            eight_latencies = []
            for index in range(EIGHT_CANDIDATE_TRIALS):
                rotated = eight_menu[index:] + eight_menu[:index]
                began = time.monotonic()
                choose(socket_path, args.model, eight_state, rotated)
                eight_latencies.append((time.monotonic() - began) * 1000)
                peak = max(peak, sample_rss(process))
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
    threshold, margin = calibrate(*evaluated["calibration"])
    held_out, decisions = evaluated["held_out"]
    stats = counts(held_out, thresholded(decisions, threshold, margin))
    latency = sorted(latencies)[math.ceil(0.95 * len(latencies)) - 1]
    eight_latency = sorted(eight_latencies)[math.ceil(0.95 * len(eight_latencies)) - 1]
    receipt.update(
        status="measured",
        calibration={"threshold": threshold, "margin": margin,
                     **counts(evaluated["calibration"][0], thresholded(evaluated["calibration"][1], threshold, margin))},
        held_out=stats, deterministic_at_matched_suggestions=baseline(held_out, stats["displayed"]),
        direct_controls_excluded_from_scoring=direct_count, invalid_responses=0,
        median_warm_ms=round(statistics.median(latencies), 2), p95_warm_ms=round(latency, 2),
        eight_candidate_trials=len(eight_latencies), eight_candidate_p95_ms=round(eight_latency, 2),
        peak_worker_rss_bytes=peak, rss_method="vmhwm" if platform.system() == "Linux" else "sampled_rss",
        meets_candidate_thresholds=(
            stats["precision"] >= 0.90 and stats["coverage"] >= 0.60 and eight_latency <= 1000
        ),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=PINS["models"], required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--target", choices=TARGETS.values(), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-liquid-evaluation", action="store_true",
                        help="Confirm this isolated evaluation is permitted by the pinned Liquid license")
    args = parser.parse_args()
    require(args.target == TARGETS.get((platform.system(), platform.machine())), "Expected native target")
    require(len(args.head) == 40 and all(char in "0123456789abcdef" for char in args.head),
            "Expected exact source revision")
    model = PINS["models"][args.model]
    require("license_sha256" not in model or args.allow_liquid_evaluation,
            "Liquid evaluation needs release-owner authorization under the pinned model license")
    runtime = PINS["runtime"]
    receipt = {"source_sha": args.head, "runtime_revision": runtime["revision"],
               "runtime_archive": runtime["archives"][args.target], "target": args.target,
               "model": args.model, "model_revision": model["revision"], "model_sha256": model["sha256"],
               "model_bytes": model["bytes"], "fixture_sha256": sha256(INCIDENTS),
               "license": model["license"], "status": "failed", "release_selectable": False,
               "measurement_scope": "Isolated CPU text scoring, without FLAN or indexing"}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="fw-decision-") as directory:
            qualify(args, receipt, Path(directory), model)
    finally:
        args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    return int(not receipt["meets_candidate_thresholds"])


if __name__ == "__main__":
    raise SystemExit(main())
