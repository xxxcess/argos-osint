#!/usr/bin/env python3
"""Measure workstation question accuracy and model use through packaged Embedded IPC."""

import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import platform
import re
import statistics
import sys
import tempfile
import time

import fleetworks_qa_pack as pack
import fleetworks_qa_smoke as qa


# Fixed questions with both exact and conversational versions of supported intents.
CASES = (
    ("cpu", "What is the current CPU utilization?", "cpu", "direct"),
    ("running-jobs", "How many jobs are running?", "running", "direct"),
    ("memory", "How much memory consumption we are at?", "memory", "generate"),
    ("memory-follow-up", "And now?", "follow_up", "generate"),
    ("resources", "What is our resource usage?", "resources", "generate"),
    ("failed-jobs", "How many jobs failed in the past 24 hours?", "failed_last_24_hours", "direct"),
    ("catalog", "Does the catalog need seeding?", "catalog", "direct"),
    ("overview", "How's everything looking?", "overview", "generate"),
    ("big-picture", "Can you give me the big picture on this workstation?", "overview", "generate"),
    ("blockers", "What needs attention?", "overview", "generate"),
    ("greeting", "Hi!", "greeting", "generate"),
    ("scope-boundary", "Can you reboot this workstation for me?", "unsupported", "decline"),
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
REPLY_INTRODUCTIONS = {"here are the current checks", "here is the current status"}
GREETING = re.compile(r"\b(?:hi|hello|hey|welcome|greetings|how (?:can|may) i help)\b", re.IGNORECASE)
PROMPT_MARKERS = ("earlier user:", "earlier assistant:", "approved observations:", "current user:", "reply:")


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
            return turn, round(1000 * (time.monotonic() - started), 1), first_draft_ms, admitted_to_worker
    raise TimeoutError(f"{case} did not finish through the packaged companion")


def metric_matches(finding):
    sample = finding.get("measurement") or {}
    value = sample.get("value")
    return (sample.get("unit") == "percent_tenths" and type(value) is int and 0 <= value <= 1000
            and f"{value // 10}.{value % 10}%" in finding["text"])


def sentences(text):
    return [" ".join(sentence.casefold().split()).rstrip(".") for sentence in text.split(". ") if sentence.strip()]


def factual_errors(turn, kind, oracle, previous_memory):
    findings = turn["findings"]
    errors = []
    if kind == "unsupported":
        if turn["state"] != "insufficient_context" or findings:
            errors.append("unsupported action was not declined without evidence")
        return errors
    if kind == "greeting":
        fallback = turn["state"] == "insufficient_context" and not turn.get("reply")
        if (turn["state"] != "answered" and not fallback) or turn.get("notice") != "greeting" or findings:
            errors.append("expected a conversational greeting without workstation claims")
        reply = (turn.get("reply") or "").casefold()
        if reply and (not GREETING.search(reply) or any(mark in reply for mark in PROMPT_MARKERS)):
            errors.append("greeting did not provide a natural acknowledgment")
        if any(phrase in reply for phrase in ("is ready", "is not ready", "jobs failed", "cpu is")):
            errors.append("greeting made a workstation claim without evidence")
        return errors
    if turn["state"] != "answered" or not findings:
        return ["no supported answer with citations"]

    sources = [(finding["source"]["kind"], finding["source"]["id"]) for finding in findings]
    for finding in findings:
        at = timestamp(finding["observed_at"])
        if at.tzinfo is None or finding["end"] - finding["start"] != len(finding["text"]):
            errors.append("citation has invalid offsets or observation time")
    if reply := turn.get("reply"):
        cited = set().union(*(sentences(finding["text"]) for finding in findings))
        if set(sentences(reply)) - cited - REPLY_INTRODUCTIONS:
            errors.append("published reply contains a sentence absent from its citations")
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
            states = (oracle["checks"].get(key), oracle["checks_after"].get(key))
            matches = any(READINESS_PHRASES[state] in finding["text"].casefold()
                          for state in states if state in READINESS_PHRASES)
            if source_kind != "readiness" or not matches:
                errors.append(f"{key} status disagrees with workstation readiness")
    return errors


def result_row(case, turn, elapsed_ms, first_draft_ms, admitted_to_worker, oracle, previous_memory):
    key, question, kind, expectation = case
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
    elif admitted_to_worker or windows or first_draft_ms is not None:
        route = "evidence_after_model"
    else:
        route = "direct_evidence" if findings else "direct_reply"

    expected_route = {
        "direct": "direct_evidence",
        "generate": "generated_reply",
        "decline": "insufficient_context",
    }[expectation]
    errors = factual_errors(turn, kind, oracle, previous_memory)
    answer = reply or " ".join(finding["text"] for finding in findings)
    parts = sentences(reply)
    style_issues = []
    if len(parts) != len(set(parts)):
        style_issues.append("model repeated a sentence")
    if any(marker in reply.casefold() for marker in PROMPT_MARKERS):
        style_issues.append("model echoed a prompt marker")
    return {"case": key, "question": question, "subject": kind, "answer": answer,
            "state": turn["state"], "notice": turn.get("notice"), "expected_route": expected_route, "route": route,
            "route_passed": route == expected_route, "checks_passed": not errors, "errors": errors,
            "style_issues": style_issues,
            "terminal_ms": elapsed_ms, "first_draft_ms": first_draft_ms, "model_windows": windows,
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
    descriptor = os.open(pointer, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
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


def benchmark(args):
    binary = qa.embedded.resolve_companion_binary(app=args.app)
    manifest_path = (
        args.app / "Contents/Resources/fleetworks-manifest.json"
        if binary.parent.name == "MacOS"
        else args.app / "share/fleetworks/fleetworks-manifest.json"
    )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    target = manifest["target"]
    with tempfile.TemporaryDirectory(prefix="fleetworks-workstation-bench-") as temporary:
        root = Path(temporary).resolve()
        state = root / "state"
        state.mkdir(mode=0o700)
        prepare_pack(args, state, target)
        # Keep the one-profile run offline even when only the Small pack is supplied.
        memory_limit = None if args.base_model_assets is not None else 1536
        with qa.embedded_qa_session(binary, state, qa_memory_mib=memory_limit) as (session, registration, request):
            context = qa.authorize(registration)
            oracle = observe_oracle(session, registration)
            started = time.monotonic()
            request("settings", {"context": context, "expected_revision": request("settings")["revision"],
                                 "enabled": True, "generator_preference": "automatic", "decision_preference": "rules_only"})
            settings = qa.until(lambda: request("settings"), lambda value: value["phase"] in {"ready", "unavailable"},
                                "Automatic did not settle its generator selection", timeout=110)
            generator = settings["generator"]
            qa.require(generator["preference"] == "automatic" and settings["phase"] == "ready"
                       and generator["effective"] in pack.CHAT_MODELS,
                       f"Automatic did not warm a verified worker ({settings.get('availability_reason')})")
            warm_ms = round(1000 * (time.monotonic() - started), 1)
            rules = check_rules_only(session, registration, request, settings)
            previous_memory = None
            rows = []
            for case in CASES:
                key, question, kind, _ = case
                case_oracle = oracle
                if kind in {"catalog", "overview"}:
                    case_oracle = {**oracle, "checks": observe_readiness(session, registration)}
                turn, elapsed, first_draft, admitted_to_worker = read_turn(request, context, key, question)
                if kind in {"catalog", "overview"}:
                    case_oracle["checks_after"] = observe_readiness(session, registration)
                row = result_row(case, turn, elapsed, first_draft, admitted_to_worker, case_oracle,
                                 previous_memory)
                rows.append(row)
                if key == "memory":
                    previous_memory = next((finding for finding in turn["findings"]
                                            if finding["source"] == {"kind": "host_metric", "id": "memory"}
                                            and metric_matches(finding)), None)
                print(f"{key}: {row['route']}, {elapsed:.1f} ms, checks={'pass' if row['checks_passed'] else 'fail'}",
                      file=sys.stderr, flush=True)
            transport = session.transport_attestation
            workers = qa.embedded_tree(session.proc.pid)
            rss = qa.embedded.read_process_rss_bytes(workers[0]) if len(workers) == 1 else None

    generated = [row for row in rows if row["route"] == "generated_reply"]
    expected_generation = [row for row in rows if row["expected_route"] == "generated_reply"]
    companion_path = binary.relative_to(args.app.resolve()).as_posix()
    return {"app_revision": manifest["revision"], "app_dirty": manifest["dirty"], "target": target,
            "companion_sha256": manifest["files"][companion_path],
            "host": {"system": platform.system(), "release": platform.release(), "arch": platform.machine()},
            "transport": transport, "generator": generator, "rules_only": rules,
            "operator_memory_limit_mib": memory_limit,
            "pack_manifest_sha256": args.manifest_sha256, "warmup_ms": warm_ms, "worker_rss_bytes": rss,
            "oracle": oracle, "summary": {
                "checks_passed": sum(row["checks_passed"] for row in rows), "cases": len(rows),
                "expected_route_passed": sum(row["route_passed"] for row in rows),
                "expected_generated": len(expected_generation),
                "worker_admitted_for_generation": sum(row["admitted_to_worker"] for row in expected_generation),
                "verified_generated": sum(row["route_passed"] for row in expected_generation),
                "generated_checks_passed": sum(row["checks_passed"] for row in generated),
                "generated_without_style_issues": sum(not row["style_issues"] for row in generated),
                "median_generated_ms": statistics.median(row["terminal_ms"] for row in generated) if generated else None,
            }, "cases": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True, help="Path to the packaged Fleetworks app with companion")
    parser.add_argument("--pack-dir", type=Path, required=True, help="Read-only directory of the verified Small pack")
    parser.add_argument("--manifest-sha256", required=True, help="The manifest SHA-256 from the trusted pack receipt")
    parser.add_argument("--base-model-assets", type=Path, help="Pinned cached Base checkpoint for offline Automatic selection")
    args = parser.parse_args()
    try:
        print(json.dumps(benchmark(args), indent=2))
    except (KeyError, OSError, ValueError) as error:
        parser.exit(1, f"fleetworks-workstation-bench: {error}\n")


if __name__ == "__main__":
    main()
