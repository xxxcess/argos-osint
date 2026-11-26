#!/usr/bin/env python3
"""Measure workstation question accuracy and model use through packaged Embedded IPC."""

import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import platform
import re
import stat
import statistics
import subprocess
import sys
import tempfile
import time

import fleetworks_platform as native_platform
import fleetworks_qa_pack as pack
import fleetworks_qa_smoke as qa


# Fixed questions with both exact and conversational versions of supported intents.
CASES = (
    ("cpu", "What is the current CPU utilization?", "cpu"),
    ("running-jobs", "How many jobs are running?", "running"),
    ("memory", "How much memory consumption we are at?", "memory"),
    ("memory-follow-up", "And now?", "follow_up"),
    ("resources", "What is our resource usage?", "resources"),
    ("failed-jobs", "How many jobs failed in the past 24 hours?", "failed_last_24_hours"),
    ("catalog", "Does the catalog need seeding?", "catalog"),
    ("overview", "How's everything looking?", "overview"),
    ("big-picture", "Can you give me the big picture on this workstation?", "overview"),
    ("blockers", "What needs attention?", "overview"),
    ("greeting", "Hi!", "greeting"),
    ("scope-boundary", "Can you reboot this workstation for me?", "unsupported"),
)

READINESS_PHRASES = {
    "ready": "is ready.",
    "not_ready": "is not ready.",
    "failed": "is not ready.",
    "unknown": "readiness is unknown.",
    "unavailable": "readiness could not be verified.",
    "not_applicable": "readiness is not applicable.",
    "degraded": "is degraded.",
    "running": "work is in progress.",
}
JOB_COUNTS = {"running": "active", "failed_last_24_hours": "failed"}
RESOURCES = {"cpu", "memory", "disk"}
REPLY_INTRODUCTIONS = ("Here are the current checks.", "Here is the current status.")
GREETING = re.compile(r"\b(?:hi|hello|hey|welcome|greetings|how (?:can|may) i help)\b", re.IGNORECASE)
PROMPT_MARKERS = ("earlier user", "earlier assistant", "current user", "approved observations:", "reply:")
SENTENCE_END = re.compile(r"[.!?]+(?:\s+|$)")
MAX_APP_MANIFEST_BYTES = 1024 * 1024
# Each fresh registration retains up to sixteen exchanges, including the measured cases.
MAX_WARMUP_TURNS = 16 - len(CASES)
MAX_ITERATIONS = 10


def timestamp(reading):
    return datetime.fromisoformat(reading.replace("Z", "+00:00"))


def observe_readiness(session, registration):
    readiness, _ = session.request_json({"op": "workstation_readiness"}, registration=registration)
    return {check["id"]: check["state"] for check in readiness["checks"]}


def observe_oracle(session, registration):
    checks = observe_readiness(session, registration)
    jobs, _ = session.request_json({"op": "jobs_list", "args": {"sort": "newest"}}, registration=registration)
    qa.require(jobs["dashboard_counts_scope"] == "unfiltered_top_level_workstation_activity",
               "The benchmark needs the registered workstation job counts")
    qa.require(checks["actor_catalog"] == "not_ready" and checks["gateway"] == "ready",
               "Unexpected readiness baseline in the isolated Embedded state")
    return {"checks": checks, "jobs": jobs["counts"]}


def read_turn(request, context, case, question):
    started = time.monotonic()
    accepted = request("turns", {"request_id": f"workstation-bench-{case}", "context": context,
                                 "question": question}, expected=(200, 201, 202))
    admitted_at = time.monotonic()
    admitted_to_worker = accepted["state"] not in qa.TERMINAL
    cursor = 0
    first_draft_ms = None
    terminal_events = 0
    deadline = started + 60
    while time.monotonic() < deadline:
        query = {"turn_id": accepted["turn_id"], "after": cursor, "wait_seconds": 1}
        page = request("events", query=query)
        qa.require(not page["gap"], f"{case} lost part of its event stream")
        for event in page["events"]:
            qa.require(event["sequence"] == cursor + 1, f"{case} event cursor skipped a sequence")
            cursor = event["sequence"]
            terminal_events += int(event["state"] in qa.TERMINAL)
            if event.get("provisional_delta") and first_draft_ms is None:
                first_draft_ms = round(1000 * (time.monotonic() - started), 1)
        turn = page["turn"]
        if turn["state"] in qa.TERMINAL and cursor == turn["cursor"]:
            qa.require(terminal_events == 1 and not turn.get("provisional_reply"),
                       f"{case} did not seal exactly one terminal turn")
            finished_at = time.monotonic()
            timings = {"admission_ms": round(1000 * (admitted_at - started), 1),
                       "post_admission_ms": round(1000 * (finished_at - admitted_at), 1),
                       "terminal_ms": round(1000 * (finished_at - started), 1),
                       "first_draft_ms": first_draft_ms}
            return turn, timings, admitted_to_worker
    raise TimeoutError(f"{case} did not finish through the packaged companion")


def metric_matches(finding):
    sample = finding.get("measurement") or {}
    value = sample.get("value")
    return (sample.get("unit") == "percent_tenths" and type(value) is int and 0 <= value <= 1000
            and f"{value // 10}.{value % 10}%" in finding["text"])


def sentences(text):
    return [" ".join(sentence.casefold().split()) for sentence in SENTENCE_END.split(text) if sentence.strip()]


def contains_prompt_marker(text):
    normalized = " ".join(text.casefold().split())
    return any(marker in normalized for marker in PROMPT_MARKERS)


def factual_errors(turn, kind, oracle, previous_memory):
    findings = turn["findings"]
    errors = []
    if kind == "unsupported":
        if turn["state"] != "answered" or not turn.get("reply") or findings:
            errors.append("expected a generated clarification without workstation evidence")
        return errors
    if kind == "greeting":
        if turn["state"] != "answered" or not turn.get("reply") or turn.get("notice") != "greeting" or findings:
            errors.append("expected a conversational greeting without workstation claims")
        reply = (turn.get("reply") or "").casefold()
        if reply and (not GREETING.search(reply) or contains_prompt_marker(reply)):
            errors.append("greeting did not provide a natural acknowledgment")
        if any(phrase in reply for phrase in ("is ready", "is not ready", "jobs failed", "cpu is")):
            errors.append("greeting made a workstation claim without evidence")
        return errors
    if turn["state"] != "answered" or not findings:
        return ["no supported answer with citations"]

    sources = [(finding["source"]["kind"], finding["source"]["id"]) for finding in findings]
    for finding in findings:
        at = timestamp(finding["observed_at"])
        start, end = finding["start"], finding["end"]
        if at.tzinfo is None or not 0 <= start < end or end - start != len(finding["text"]):
            errors.append("citation has invalid offsets or observation time")
    if reply := turn.get("reply"):
        cited = " ".join(finding["text"] for finding in findings)
        if reply != cited and not any(reply == f"{intro} {cited}" for intro in REPLY_INTRODUCTIONS):
            errors.append("published reply differs from its complete cited passages")
    if kind in RESOURCES or kind in {"resources", "follow_up"}:
        expected = (
            {("host_metric", key) for key in RESOURCES}
            if kind == "resources"
            else {("host_metric", "memory" if kind == "follow_up" else kind)}
        )
        expected_count = 2 if kind == "follow_up" else len(expected)
        if set(sources) != expected or len(sources) != expected_count or not all(map(metric_matches, findings)):
            errors.append("resource percentage or source disagrees with the frozen measurement")
        if kind == "follow_up":
            if (previous_memory is None or len(findings) != 2
                    or findings[1]["observed_at"] != previous_memory["observed_at"]
                    or findings[1].get("measurement") != previous_memory.get("measurement")
                    or timestamp(findings[0]["observed_at"]) <= timestamp(previous_memory["observed_at"])):
                errors.append("follow-up did not separate the current memory reading from the prior one")
    elif kind in JOB_COUNTS:
        expected = oracle["jobs"][JOB_COUNTS[kind]]
        measurement = findings[0].get("measurement") or {}
        if (sources != [("job_count", kind)] or measurement.get("unit") != "jobs"
                or measurement.get("value") != expected
                or (kind == "failed_last_24_hours" and "24 hours" not in findings[0]["text"])):
            errors.append("job count or reporting window disagrees with scoped activity")
    else:
        if ("readiness", "actor_catalog") not in sources or (kind == "catalog" and len(sources) != 1):
            errors.append("answer missed the unseeded catalog")
        for finding, (source_kind, key) in zip(findings, sources):
            states = (oracle["checks"].get(key), oracle.get("checks_after", {}).get(key))
            matches = any(READINESS_PHRASES[state] in finding["text"].casefold()
                          for state in states if state in READINESS_PHRASES)
            if source_kind != "readiness" or not matches:
                errors.append(f"{key} status disagrees with workstation readiness")
    return errors


def result_row(case, turn, timings, admitted_to_worker, oracle, previous_memory):
    key, question, kind = case
    usage = turn.get("context") or {}
    windows = usage.get("model_windows", 0)
    reply = turn.get("reply") or ""
    findings = turn["findings"]
    if kind == "greeting" and turn["state"] == "insufficient_context" and turn.get("notice") == "greeting":
        route = "greeting_notice"
    elif turn["state"] != "answered":
        route = turn["state"]
    elif reply and windows:
        route = "generated_reply"
    elif admitted_to_worker or windows or timings["first_draft_ms"] is not None:
        route = "evidence_after_model"
    else:
        route = "direct_evidence" if findings else "direct_reply"

    expected_route = "generated_reply"
    errors = factual_errors(turn, kind, oracle, previous_memory)
    answer = reply or " ".join(finding["text"] for finding in findings)
    parts = sentences(reply)
    style_issues = []
    if kind == "greeting" and len(parts) != len(set(parts)):
        style_issues.append("model repeated a sentence")
    if contains_prompt_marker(reply):
        style_issues.append("model echoed a prompt marker")
    return {"case": key, "question": question, "subject": kind, "answer": answer,
            "state": turn["state"], "notice": turn.get("notice"), "expected_route": expected_route, "route": route,
            "route_passed": route == expected_route, "checks_passed": not errors, "errors": errors,
            "style_issues": style_issues,
            **timings, "model_windows": windows,
            "admitted_to_worker": admitted_to_worker,
            "effective_model": turn.get("model_target"), "citations": findings}


def prepare_pack(args, state, target):
    source = args.pack_dir.expanduser().absolute()
    manifest = pack.verify_directory(source, target, args.manifest_sha256)
    qa.require(pack.model_for_pack(manifest) == "flan-t5-small", "Use the standard offline pack containing Small")
    if args.base_model_assets is not None:
        pack.materialize_model_assets(args.base_model_assets, state, "flan-t5-base")

    # The exact verified pack is reused read-only. The registration, state and
    # any extra model assets are owned solely by this benchmark run.
    pointer = state / "qa-pack.json"
    descriptor = os.open(pointer, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump({"root": str(source.resolve()), "manifest_sha256": args.manifest_sha256}, output)


def check_rules_only(session, registration, request, settings):
    decision = settings["decision"]
    qa.require(decision["preference"] == decision["effective"] == "rules_only",
               "The benchmark requires Rules only as the saved and effective decision policy")
    qa.require(request("latest") is None, "Expected an empty benchmark transcript")
    source = {"kind": "readiness", "id": "actor_catalog"}
    result, _ = session.request_json({"op": "qa_related_activity", "args": {"source": source}},
                                     registration=registration)
    qa.require(result == {"status": "found", "target": source, "reason": "current_status"},
               "Rules only did not resolve the catalog check to its current status")
    qa.require(request("latest") is None, "Rules only lookup created an Assistant exchange")
    return {"settings": decision, "lookup": result}


def verify_companion(app):
    binary = qa.embedded.resolve_companion_binary(app=app)
    manifest_path = (
        app / "Contents/Resources/fleetworks-manifest.json"
        if binary.parent.name == "MacOS"
        else app / "share/fleetworks/fleetworks-manifest.json"
    )
    qa.require(binary.resolve() == binary and manifest_path.resolve() == manifest_path,
               "Packaged companion and manifest must use unlinked paths")
    descriptor = os.open(manifest_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        info = os.fstat(source.fileno())
        qa.require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= MAX_APP_MANIFEST_BYTES,
                   "App manifest must be a bounded regular file")
        raw = source.read(MAX_APP_MANIFEST_BYTES + 1)
    qa.require(len(raw) <= MAX_APP_MANIFEST_BYTES, "App manifest exceeds its size bound")
    manifest = json.loads(raw, object_pairs_hook=pack.unique_pairs)
    qa.require(isinstance(manifest, dict) and isinstance(manifest.get("revision"), str)
               and type(manifest.get("dirty")) is bool, "Expected an app manifest with revision metadata")
    target = manifest.get("target")
    machine = native_platform.ARCHITECTURES.get(platform.machine().lower())
    targets = {"Darwin": native_platform.DARWIN_NATIVE_TARGETS, "Linux": native_platform.LINUX_NATIVE_TARGETS}
    qa.require(isinstance(target, str) and machine is not None
               and targets.get(platform.system(), {}).get(target) == machine,
               "App manifest target does not match the benchmark host")
    files = manifest.get("files")
    expected = files.get(binary.relative_to(app).as_posix()) if isinstance(files, dict) else None
    qa.require(pack.is_digest(expected), "App manifest lacks a companion SHA-256")
    digest = pack.sha256(binary)
    qa.require(digest == expected, "Companion SHA-256 does not match the app manifest")
    return binary, manifest, digest


def peak_worker_rss(pid):
    try:
        status = Path(f"/proc/{pid}/status").read_text(encoding="utf-8")
    except OSError:
        return None
    peak = re.search(r"^VmHWM:\s+([0-9]+) kB$", status, re.M)
    return int(peak[1]) * 1024 if peak else None


def run_iteration(args, binary, target):
    with tempfile.TemporaryDirectory(prefix="fleetworks-workstation-bench-") as temporary:
        root = Path(temporary).resolve()
        state = root / "state"
        state.mkdir(mode=0o700)
        prepare_pack(args, state, target)
        # Keep the one-profile run offline even when only the Small pack is supplied.
        memory_limit = None if args.base_model_assets is not None else 1536
        with qa.embedded_qa_session(binary, state, qa_memory_mib=memory_limit,
                                    isolated_root=root) as (session, registration, request):
            context = qa.authorize(registration)
            defaults = request("settings")
            qa.require(defaults["generator"]["preference"] == "automatic"
                       and defaults["decision"]["preference"] == "rules_only",
                       "Fresh settings must default to Automatic and Rules only")
            oracle = observe_oracle(session, registration)
            started = time.monotonic()
            request("settings", {"context": context, "expected_revision": defaults["revision"],
                                 "enabled": True})
            settings = qa.until(lambda: request("settings"), lambda value: value["phase"] in {"ready", "unavailable"},
                                "Automatic did not settle its generator selection", timeout=110)
            generator = settings["generator"]
            qa.require(generator["preference"] == "automatic" and settings["phase"] == "ready"
                       and generator["effective"] in pack.CHAT_MODELS,
                       f"Automatic did not warm a verified worker ({settings.get('availability_reason')})")
            warm_ms = round(1000 * (time.monotonic() - started), 1)
            rules = check_rules_only(session, registration, request, settings)
            for warmup in range(args.warmup_turns):
                read_turn(request, context, f"warmup-{warmup}", "Hi!")
            previous_memory = None
            rows = []
            peak_rss = None
            for case in CASES:
                key, question, kind, _ = case
                case_oracle = oracle
                if kind in {"catalog", "overview"}:
                    case_oracle = {**oracle, "checks": observe_readiness(session, registration)}
                turn, timings, admitted_to_worker = read_turn(request, context, key, question)
                if kind in {"catalog", "overview"}:
                    case_oracle["checks_after"] = observe_readiness(session, registration)
                row = result_row(case, turn, timings, admitted_to_worker, case_oracle, previous_memory)
                rows.append(row)
                if key == "memory":
                    previous_memory = next((finding for finding in turn["findings"]
                                            if finding["source"] == {"kind": "host_metric", "id": "memory"}
                                            and metric_matches(finding)), None)
                if platform.system() == "Linux":
                    workers = qa.embedded_tree(session.proc.pid)
                    if len(workers) == 1 and (sample := peak_worker_rss(workers[0])) is not None:
                        peak_rss = max(peak_rss or 0, sample)
                print(f"{key}: {row['route']}, {row['terminal_ms']:.1f} ms, checks={'pass' if row['checks_passed'] else 'fail'}",
                      file=sys.stderr, flush=True)
            transport = session.transport_attestation
            workers = qa.embedded_tree(session.proc.pid)
            rss = qa.embedded.read_process_rss_bytes(workers[0]) if len(workers) == 1 else None

    return {"transport": transport, "generator": generator, "rules_only": rules,
            "operator_memory_limit_mib": memory_limit,
            "warmup_ms": warm_ms, "worker_rss_bytes": rss, "worker_peak_rss_bytes": peak_rss,
            "oracle": oracle, "cases": rows}


def summarize(rows):
    generated = [row for row in rows if row["route"] == "generated_reply"]
    verified = [row for row in generated if row["checks_passed"] and not row["style_issues"]]
    expected_generation = [row for row in rows if row["expected_route"] == "generated_reply"]
    return {
        "checks_passed": sum(row["checks_passed"] for row in rows), "cases": len(rows),
        "expected_route_passed": sum(row["route_passed"] for row in rows),
        "expected_generated": len(expected_generation),
        "worker_admitted_for_generation": sum(row["admitted_to_worker"] for row in expected_generation),
        "verified_generated": sum(row["expected_route"] == "generated_reply" for row in verified),
        "verified_operational_model_replies": sum(row["subject"] != "greeting" for row in verified),
        "verified_conversational_model_replies": sum(row["subject"] == "greeting" for row in verified),
        "generated_checks_passed": sum(row["checks_passed"] for row in generated),
        "generated_without_style_issues": sum(not row["style_issues"] for row in generated),
        "median_generated_ms": statistics.median(row["terminal_ms"] for row in generated) if generated else None,
    }


def benchmark(args):
    app = args.app.expanduser().resolve()
    binary, manifest, digest = verify_companion(app)
    runs = []
    for iteration in range(1, args.iterations + 1):
        run = run_iteration(args, binary, manifest["target"])
        for row in run["cases"]:
            row["iteration"] = iteration
        runs.append(run)
    rows = [row for run in runs for row in run["cases"]]
    evidence = {key: value for key, value in runs[0].items() if key != "cases"}
    if len(runs) > 1:
        evidence = {"iterations": [{key: value for key, value in run.items() if key != "cases"} for run in runs]}
    return {"app_revision": manifest["revision"], "app_dirty": manifest["dirty"], "target": manifest["target"],
            "companion_sha256": digest,
            "host": {"system": platform.system(), "release": platform.release(), "arch": platform.machine()},
            "pack_manifest_sha256": args.manifest_sha256, "warmup_turns": args.warmup_turns,
            **evidence, "summary": summarize(rows), "cases": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True, help="Path to the packaged Fleetworks app with companion")
    parser.add_argument("--pack-dir", type=Path, required=True, help="Read-only directory of the verified Small pack")
    parser.add_argument("--manifest-sha256", required=True, help="The manifest SHA-256 from the trusted pack receipt")
    parser.add_argument("--base-model-assets", type=Path, help="Pinned cached Base checkpoint for offline Automatic selection")
    parser.add_argument("--warmup-turns", type=int, choices=range(MAX_WARMUP_TURNS + 1), default=0,
                        help="Unmeasured greetings before each measured iteration")
    parser.add_argument("--iterations", type=int, choices=range(1, MAX_ITERATIONS + 1), default=1,
                        help="Number of independently registered benchmark runs")
    parser.add_argument("--fail-on-error", action="store_true",
                        help="Exit nonzero on a factual, route, or reply-style failure")
    args = parser.parse_args()
    try:
        result = benchmark(args)
        print(json.dumps(result, indent=2))
    except (KeyError, OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"fleetworks-workstation-bench: {error}\n")
    if args.fail_on_error and any(
        not row["checks_passed"] or not row["route_passed"] or row["style_issues"] for row in result["cases"]
    ):
        parser.exit(1, "fleetworks-workstation-bench: one or more cases failed\n")


if __name__ == "__main__":
    main()
