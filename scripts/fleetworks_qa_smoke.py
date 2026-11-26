#!/usr/bin/env python3
"""Exercise the real Inbox QA pack through a packaged Embedded companion."""

import argparse
from contextlib import contextmanager
from datetime import datetime
import hashlib
from itertools import islice
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time

import fleetworks_local_backend_smoke as embedded
import fleetworks_qa_pack as pack

ROOT = Path(__file__).resolve().parents[1]
TERMINAL = {"answered", "insufficient_context", "canceled", "failed"}
SUMMARY_QUESTION = "What is the current workstation readiness?"
CONTEXT_WINDOW_TOKENS = 512
FIXTURE_MODEL_MEMORY_MIB = 1536
ANSI = re.compile(r"\x1b\[[0-9;]*m")
DIAGNOSTIC = re.compile(r'\b(qa_(?:warm|replica|queue|turn|settings)_stage)="?([a-z0-9_]{1,40})\b')
DIAGNOSTIC_DETAIL = re.compile(
    r'\b(qa_warm_reason|qa_warm_elapsed_ms|landlock_abi|sanitized_os_error|worker_exit_code|worker_signal)'
    r'="?(Some\(-?\d{1,9}\)|-?\d{1,9}|abandoned|landlock_abi)(?=["\s,|]|$)'
)
FIXED_DIAGNOSTIC_DETAIL = re.compile(
    r'\b(worker_panic_source|worker_panic_line|worker_panic_cause|qa_settings_error)="?([a-z0-9_]{1,40})(?=["\s,|]|$)'
)
SAFE_REJECTION = {
    "Backend inference is off": "inference_off",
    "Backend inference has no ready worker": "no_ready_worker",
    "Backend inference is unavailable": "coordinator_unavailable",
    "Current backend health could not be verified": "fresh_health_unavailable",
    "Inbox inference is busy": "busy",
}
CATALOG_TIMEOUT_SECONDS = 43 * 60
CATALOG_STAGE_TIMEOUT_SECONDS = 4 * 60
CHECKPOINT_POLL_SECONDS = 10
INDEX_PROGRESS_POLL_SECONDS = 20
CATALOG_PROGRESS_MILESTONES = 6
CATALOG_ROWS = 94_987
ROCKSDB_OPTIONS = {
    "write_buffer_size": 8 * 1024 * 1024,
    "max_write_buffer_number": 2,
    "min_write_buffer_number_to_merge": 1,
    "max_background_jobs": 2,
    "max_subcompactions": 1,
}
ROCKSDB_BLOCK_CACHE_BYTES = 16 * 1024 * 1024
SMAPS_ROLLUP_FIELDS = {
    "Rss", "Pss_Anon", "Pss_File", "Pss_Shmem", "Private_Clean", "Private_Dirty",
    "Shared_Clean", "Shared_Dirty", "Anonymous", "Swap",
}
TERMINAL_JOB_STATES = {"completed", "failed", "cancelled", "canceled", "stopped", "skipped", "superseded"}
ACTIVE_JOB_STATES = {"queued", "running", "waiting", "reconciling", "retry_scheduled"}
SAFE_CATALOG_FAILURES = {
    "setup_worker_resource_pressure", "setup_dataset_resource_budget",
    "setup_worker_memory_pressure", "setup_worker_disk_pressure", "setup_worker_pressure_unavailable",
    "setup_interrupted", "native_domain_operation_failed",
}
SAFE_INDEX_REASONS = {
    "index_building", "index_catching_up", "index_generation_mismatch", "index_ingest_paused",
    "index_not_built", "index_not_ready", "index_reader_visible", "index_rebuild_correction_required",
    "index_rebuild_interrupted", "index_recovery_required", "index_source_mismatch",
    "index_source_not_started", "index_status_unavailable",
}
SAFE_INDEX_STATES = {
    "degraded", "indexing", "not_applicable", "not_built", "not_ready", "paused", "ready", "running",
    "stale", "throttled", "unavailable", "unknown",
}
INDEX_EVENT = re.compile(r'\bcode="?embedded_actor_index_(batch_elapsed|pressure_transition|merge_planned)"?(?=$|[\s,])')
INDEX_BATCH_FIELD = re.compile(r'\b(prepare_ms|write_ms|ack_ms|mutations)=([0-9]{1,9})(?=$|[\s,])')
INDEX_PRESSURE_FIELD = re.compile(r'\b(resident_bytes|disk_free_bytes)=([0-9]{1,20})(?=$|[\s,])')
INDEX_PRESSURE_REASON = re.compile(r'\breason="?([a-z_]{1,40})"?(?=$|[\s,])')
INDEX_PRESSURE_PAUSED = re.compile(r'\bpaused=(true|false)(?=$|[\s,])')
INDEX_MERGE_FIELD = re.compile(r'\b(inputs|documents)=([0-9]{1,9})(?=$|[\s,])')
INDEX_BATCH_DURATIONS = ("prepare_ms", "write_ms", "ack_ms")
CATALOG_EVENT_MILESTONES = 12
INDEX_PRESSURE_REASONS = {
    "memory_high_water", "disk_high_water", "pressure_measurement_unavailable",
    "query_latency_high_water", "commit_latency_high_water", "latency_probe_due", "none",
}
SEED_EVENT = re.compile(r'\bcode="?embedded_(actor_seed_batch_elapsed|actor_seed_phase_elapsed|backend_shutdown_phase)"?(?=$|[\s,])')
SEED_BATCH_FIELD = re.compile(
    r'\b(capacity_wait_us|capacity_in_commit_us|commit_us|plan_us|canonical_write_us|alias_write_us|'
    r'derived_write_us|rows|checkpoint|transaction_retries)=([0-9]{1,20})(?=$|[\s,])'
)
SEED_PHASE_ELAPSED = re.compile(r'\belapsed_us=([0-9]{1,20})(?=$|[\s,])')
CATALOG_PHASE = re.compile(r'\bphase="?([a-z_]{1,40})"?(?=$|[\s,])')
SEED_PHASES = {"admission", "csv_spool_validation", "job_progress", "job_log", "final_progress"}
SHUTDOWN_PHASES = {
    "ipc_drained", "health_stopped", "qa_stopped", "context_released",
    "runtime_shutdown_started", "runtime_shutdown_finished",
}
ROCKSDB_SHUTDOWN_INFO = re.compile(
    r'(?:^|\s)INFO[ \t]+surrealdb::core::kvs::rocksdb:[ \t]+'
    r'(?:Waiting for in-flight compactions to drain \(timeout: ([0-9]{1,4})s\)|Cancelling background work)$'
)
SHUTDOWN_TASK_LIMIT = 64
SHUTDOWN_WAIT_CHANNELS = {
    "bit_wait_io", "do_epoll_wait", "do_wait", "ep_poll", "filemap_fdatawait_range",
    "folio_wait_bit_common", "futex_wait", "futex_wait_queue", "futex_wait_queue_me",
    "hrtimer_nanosleep", "io_schedule", "io_schedule_timeout", "pipe_read", "pipe_write",
    "poll_schedule_timeout", "schedule_timeout", "wait_for_completion", "wait_woken",
}


def require(condition, description):
    if not condition:
        raise ValueError(description)


def authorize(registration):
    return {
        "registration_id": registration["registration_id"],
        "expected_registration_revision": registration["registration_revision"],
        "expected_context_revision": registration["context_revision"],
    }


def until(probe, ready, description, *, timeout=90, interval=0.25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if ready(value):
            return value
        time.sleep(interval)
    raise TimeoutError(description)


def report_diagnostics(logs):
    stages = set()
    lines = ANSI.sub("", logs).splitlines()
    for line in lines:
        for stage, code in DIAGNOSTIC.findall(line):
            details = DIAGNOSTIC_DETAIL.findall(line) + FIXED_DIAGNOSTIC_DETAIL.findall(line)
            stages.add((stage, code, tuple(sorted(details))))
    print(json.dumps({
        "qa_failure_stages": [
            {"stage": stage, "code": code, **dict(details)} for stage, code, details in sorted(stages)
        ],
        "diagnostic_log_lines": len(lines),
    }), file=sys.stderr)


def cited_finding(turn, kind, source_id, *, count=1):
    require(turn["state"] == "answered" and len(turn["findings"]) == count,
            f"{source_id} did not produce {count} cited observations")
    finding = turn["findings"][0]
    require(finding["source"] == {"kind": kind, "id": source_id},
            f"{source_id} answer cited a different source")
    require(0 <= finding["start"] < finding["end"]
            and finding["end"] - finding["start"] == len(finding["text"])
            and datetime.fromisoformat(finding["observed_at"]).tzinfo is not None,
            f"{source_id} answer lacks valid offsets or an observation time")
    return finding


def require_context(turn, *, inferred, window=CONTEXT_WINDOW_TOKENS):
    usage = turn.get("context")
    require(isinstance(usage, dict) and usage.get("window_tokens") == window,
            "terminal turn lacks its model context window")
    peak = usage.get("peak_input_tokens")
    windows = usage.get("model_windows")
    require(type(peak) is int and type(windows) is int
            and (0 < peak <= window and 0 < windows <= 48 if inferred else peak == windows == 0),
            "terminal turn has incorrect model context usage")
    return usage


def settled_turn(request, context, case, question, *, source=None, mode="direct"):
    payload = {"request_id": f"inbox-qa-smoke-{case}", "context": context, "question": question}
    if source is not None:
        payload["source"] = source
    accepted = request("turns", payload, expected=(200, 201, 202))
    query = {"turn_id": accepted["turn_id"], "after": 0, "wait_seconds": 0}
    inferred = mode != "direct"
    if inferred:
        accepted = until(lambda: request("events", query=query)["turn"],
                         lambda turn: turn["state"] in TERMINAL,
                         f"{case} did not complete the local inference turn", timeout=45)
        if mode == "dialogue":
            require(accepted["state"] == "answered" and accepted.get("reply") and not accepted["findings"],
                    f"{case} did not produce bounded conversational text")
        else:
            require(accepted["state"] == "answered" and accepted["findings"],
                    f"{case} did not produce a grounded explanation")
    else:
        require(accepted["state"] in TERMINAL, f"{case} did not finish within admission")
    require(type(accepted.get("response_time_ms")) is int and accepted["response_time_ms"] >= 0,
            f"{case} terminal turn omitted its response duration")
    # Generation-preferred operational turns may finish with the same typed
    # facts if generation cannot produce a verifiable reply.
    used_model = inferred and (mode != "preferred" or bool((accepted.get("context") or {}).get("model_windows")))
    usage = require_context(accepted, inferred=used_model)
    if used_model:
        require(usage["model_windows"] == 1, f"{case} did not use a single model input window")
    page = request("events", query=query)
    require(page["turn"] == accepted and not page["gap"], f"{case} admission differed from its event snapshot")
    require(not accepted.get("provisional_reply") and not any(event.get("provisional_delta") for event in page["events"]),
            f"{case} retained provisional text after the terminal event")
    require([event["sequence"] for event in page["events"]] == list(range(1, accepted["cursor"] + 1)),
            f"{case} event cursor skipped a sequence")
    require([event["sequence"] for event in page["events"] if event["state"] in TERMINAL] == [accepted["cursor"]],
            f"{case} did not emit exactly one terminal event")
    require([event["finding"] for event in page["events"] if event["finding"]] == accepted["findings"],
            f"{case} event evidence differs from the frozen answer")
    require(request("latest") == accepted, f"{case} latest changed the frozen answer")
    require(request("turns", payload, expected=(200, 201, 202)) == accepted, f"{case} replay changed the answer")
    cancel = {"context": context, "turn_id": accepted["turn_id"]}
    require(request("cancel", cancel) == accepted, f"{case} Stop changed a grounded terminal answer")
    resume = {**query, "after": accepted["cursor"]}
    require(request("events", query=resume) == {"turn": accepted, "events": [], "gap": False},
            f"{case} terminal replay emitted another event")

    return accepted


def cited_metric(turn, source_id, label, *, previous=None):
    finding = cited_finding(turn, "host_metric", source_id, count=2 if previous else 1)
    measurement = finding["measurement"]
    value = measurement["value"]
    subject = f"API host {label}"
    require(type(value) is int and 0 <= value <= 1000
            and measurement == {"subject": subject, "unit": "percent_tenths", "value": value},
            f"{source_id} evidence does not contain a valid utilization measurement")
    whole, tenths = divmod(value, 10)
    require(finding["label"] == subject
            and finding["text"] == f"The {subject} is {whole}.{tenths}%.",
            f"{source_id} answer differs from its frozen measurement")
    if previous:
        prior = turn["findings"][1]
        prior_whole, prior_tenths = divmod(previous["value"], 10)
        also = "also " if previous["value"] == value else ""
        require(prior["source"] == finding["source"] and prior["label"] == "Previous observation"
                and prior["observed_at"] == previous["observed_at"]
                and prior["measurement"] == {**measurement, "value": previous["value"]}
                and prior["text"] == f"The previous observation was {also}{prior_whole}.{prior_tenths}%.",
                f"{source_id} did not cite the immediately preceding comparable observation")
    return {"value": value, "unit": measurement["unit"], "observed_at": finding["observed_at"]}


def cited_running_jobs(turn, active_jobs):
    finding = cited_finding(turn, "job_count", "running")
    job_noun = "job" if active_jobs == 1 else "jobs"
    require(finding["text"] == f"This workstation has {active_jobs} running {job_noun}."
            and finding["measurement"] == {
                "subject": "Running workstation jobs", "unit": "jobs", "value": active_jobs,
            },
            "Inbox running-job count differs from the registered workstation dashboard")


def cancel_running_embedded_turn(request, registration, model, *, draft=False):
    context = authorize(registration)
    before = request("settings")
    require(not before["enabled"] and before["phase"] == "off", "Gated fixture must start with QA off")
    request("settings", {"context": context, "expected_revision": before["revision"], "enabled": True})
    until(lambda: request("settings"), lambda value: value["phase"] == "ready",
          "Gated worker did not warm", timeout=110)
    active = model()
    require(len(active) == 1, "Expected one isolated gated worker")
    worker, = active
    case = "draft" if draft else "finding"
    turn = request("turns", {"request_id": f"inbox-qa-smoke-cancel-{case}", "context": context,
                             "question": "hi" if draft else SUMMARY_QUESTION}, expected=(200, 201, 202))
    turn_id = turn["turn_id"]
    query = {"turn_id": turn_id, "after": 0, "wait_seconds": 0}
    page = until(lambda: request("events", query=query),
                 lambda value: value["turn"].get("provisional_reply" if draft else "findings")
                 or value["turn"]["state"] in TERMINAL,
                 f"Gated worker did not deliver a {case} before Stop", timeout=90)
    held = page["turn"]
    require(not page["gap"] and held["state"] == "running" and held["turn_id"] == turn_id
            and model() == [worker], f"Expected one worker while the Running {case} was paused")
    if draft:
        deltas = [event["provisional_delta"] for event in page["events"] if event.get("provisional_delta")]
        require(len(deltas) == 1 and held.get("provisional_reply") == deltas[0] and not held["findings"],
                "The first conversational delta was not visible in the Running turn")
    else:
        require(len(held["findings"]) == 1 and any(event.get("finding") for event in page["events"]),
                "The first frozen finding was not visible in the Running turn")
    canceled = request("cancel", {"context": context, "turn_id": turn_id})
    require(canceled["turn_id"] == turn_id and canceled["state"] == "canceled"
            and canceled["cursor"] == held["cursor"] + 1 and canceled["findings"] == held["findings"]
            and not canceled.get("provisional_reply") and not canceled.get("reply"),
            "Stop did not seal the held turn, clear the draft, and preserve committed findings")
    if model() == [worker]:
        try:
            os.kill(worker, signal.SIGUSR1)
        except ProcessLookupError:
            # The coordinator may already have reaped the canceled worker.
            pass
    result = request("events", query=query)
    terminal = [event for event in result["events"] if event["state"] in TERMINAL]
    require(not result["gap"] and result["turn"] == canceled and canceled["turn_id"] == turn_id
            and canceled["state"] == "canceled" and len(terminal) == 1
            and terminal[0]["state"] == "canceled" and terminal[0]["sequence"] == canceled["cursor"]
            and not any(event.get("provisional_delta") for event in result["events"])
            and [event["sequence"] for event in result["events"]] == list(range(1, canceled["cursor"] + 1)),
            "Stop did not retain exactly one canceled terminal event for the Running turn")
    replay = request("events", query={**query, "after": canceled["cursor"]})
    require(not replay["gap"] and not replay["events"] and replay["turn"] == canceled,
            "Canceled terminal replay changed or duplicated an event")
    if draft:
        return {"first_draft_while_running": True, "cancelled_draft_terminal_events": len(terminal),
                "cancelled_draft_terminal_state": canceled["state"], "cancelled_draft_scrubbed": True}
    return {"cancelled_terminal_events": len(terminal), "cancelled_terminal_state": canceled["state"]}


def ready_model(request, target, *, updated=None, stages=None):
    if stages is None:
        stages = set()

    def observe(value):
        if value.get("startup_stage"):
            stages.add(value["startup_stage"])
        return value

    if updated is not None:
        observe(updated)
    return until(lambda: observe(request("settings")),
                 lambda value: value["phase"] == "ready" and value["model_target"] == target,
                 f"{target} QA worker did not warm", timeout=110, interval=0.1)


def stop_model(request, registration, model, settings):
    request("settings", {"context": authorize(registration), "expected_revision": settings["revision"],
                         "enabled": False})
    until(lambda: request("settings"), lambda value: value["phase"] == "off",
          "QA did not finish unloading", timeout=40)
    require(not model(), "Off retained a live model process")


def exercise(request, registration, model, readiness, running_jobs):
    """Run the QA lifecycle over the packaged companion's private socket."""
    before = request("settings")
    require(not before["enabled"] and before["phase"] == "off", "QA did not default to Off")
    require(not model(), "Off retained a live model process")
    require(request("latest") is None, "Off exposes an exchange")
    turn = {"request_id": "inbox-qa-smoke-1", "context": authorize(registration), "question": SUMMARY_QUESTION}
    request("turns", turn, expected=503)

    revision = before["revision"]
    started = time.monotonic()
    updated = request("settings", {"context": authorize(registration), "expected_revision": revision, "enabled": True})
    stages = set()
    try:
        current = ready_model(request, "flan-t5-small", updated=updated, stages=stages)
    except TimeoutError:
        current = request("settings")
        phase = current.get("phase")
        if phase not in {"off", "starting", "ready", "stopping", "unavailable"}:
            phase = "unknown"
        raise TimeoutError(f"QA worker did not warm (last phase {phase})") from None
    ready_ms = round((time.monotonic() - started) * 1000, 1)
    active = model()
    require(len(active) == 1, "Ready did not retain one live model process")
    worker_rss_bytes = embedded.read_process_rss_bytes(active[0])
    require(readiness(), "QA warm-up blocked normal workstation readiness")

    started = time.monotonic()
    accepted = request("turns", turn, expected=(200, 201, 202))
    admission_ms = round((time.monotonic() - started) * 1000, 1)
    turn_id = accepted["turn_id"]
    replay = request("turns", turn, expected=(200, 201, 202))
    require(replay["turn_id"] == turn_id, "the replay admitted another turn")
    request("turns", {**turn, "question": "Different question"}, expected=409)
    cursor = 0
    terminal_events = 0
    observed_states = set()
    first_finding_ms = None
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        page = request("events", query={"turn_id": turn_id, "after": cursor, "wait_seconds": 4})
        snapshot = page["turn"]
        require(snapshot["turn_id"] == turn_id and not page["gap"], "event cursor could not resume")
        for event in page["events"]:
            require(event["sequence"] == cursor + 1, "event sequence is not consecutive")
            cursor = event["sequence"]
            if first_finding_ms is None and event.get("finding"):
                first_finding_ms = round((time.monotonic() - started) * 1000, 1)
            terminal_events += int(event["state"] in TERMINAL)
            if event["state"] in TERMINAL | {"queued", "running"}:
                observed_states.add(event["state"])
        if snapshot["state"] in TERMINAL:
            break
    else:
        raise TimeoutError("QA turn did not finish")
    completed_ms = round((time.monotonic() - started) * 1000, 1)
    if snapshot["state"] != "answered" or first_finding_ms is None:
        print(json.dumps({"qa_smoke_stage": "first_turn_terminal", "state": snapshot["state"],
                          "observed_states": sorted(observed_states), "finding_count": len(snapshot["findings"]),
                          "model_warm_ms": ready_ms, "admission_ms": admission_ms,
                          "completion_ms": completed_ms}), file=sys.stderr)
    require(terminal_events == 1 and snapshot["cursor"] == cursor, "terminal delivery was not exactly once")
    require(snapshot["state"] == "answered" and first_finding_ms is not None and not snapshot.get("provisional_reply"),
            "real local checkpoint did not seal a cited answer")
    model_context = require_context(snapshot, inferred=True)
    require(model_context["model_windows"] == 1, "operational reply did not use a single model input window")
    require(1 <= len(snapshot["findings"]) <= 3, "QA finding count exceeds its contract")
    for finding in snapshot["findings"]:
        require(0 <= finding["start"] < finding["end"]
                and finding["end"] - finding["start"] == len(finding["text"])
                and finding["observed_at"] and finding["source"]["id"]
                and finding["source"]["kind"] == "readiness", "invalid readiness citation")
        require(re.search(r"\b\d{4}-\d{2}-\d{2}|\b\d{2}:\d{2}:\d{2}", finding["text"]) is None,
                "the real-model summary leaked an observation timestamp into its answer")
    require(request("latest") == snapshot, "latest exchange differs from event snapshot")
    resumed = request("events", query={"turn_id": turn_id, "after": cursor, "wait_seconds": 0})
    require(not resumed["events"] and resumed["turn"] == snapshot, "terminal replay duplicated an event")
    require(readiness(), "ordinary readiness failed after QA inference")
    print(json.dumps({"qa_smoke_stage": "first_turn_answered", "model_warm_ms": ready_ms,
                      "admission_ms": admission_ms, "first_cited_finding_ms": first_finding_ms,
                      "completion_ms": completed_ms, "warm_model_rss_bytes": worker_rss_bytes}),
          file=sys.stderr)

    follow_up = request("turns", {**turn, "request_id": "inbox-qa-smoke-2"}, expected=(200, 201, 202))
    follow_up_id = follow_up["turn_id"]
    require(follow_up_id != turn_id, "the next exchange reused the previous turn")
    next_turn = until(
        lambda: request("events", query={"turn_id": follow_up_id, "after": 0, "wait_seconds": 0})["turn"],
        lambda value: value["state"] in TERMINAL, "fresh-evidence turn did not finish")
    require(next_turn["state"] == "answered" and next_turn["findings"], "fresh evidence did not support the follow-up")
    require_context(next_turn, inferred=True)

    def observed(finding):
        return datetime.fromisoformat(finding["observed_at"].replace("Z", "+00:00"))

    require(min(map(observed, next_turn["findings"])) > max(map(observed, snapshot["findings"])),
            "the follow-up reused an observation from before the previous terminal turn")

    context = authorize(registration)

    preferred_model_fallback_count = 0

    def ask(case, question, *, mode="direct"):
        nonlocal preferred_model_fallback_count
        turn = settled_turn(request, context, case, question, mode=mode)
        if mode == "preferred" and not turn.get("reply"):
            preferred_model_fallback_count += 1
        return turn

    def check_status_group(question, group, prior_observation):
        status_turn = ask(f"{group}-status-checks", question, mode="preferred")
        findings = status_turn["findings"]
        require(status_turn["state"] == "answered" and status_turn["question"] == question
                and 1 <= len(findings) <= 3,
                f"{group} status checks did not return a bounded readiness answer")
        summary = findings[0]
        require(summary["source"] == {"kind": "readiness", "id": "workstation_status_checks"}
                and group in summary["text"].casefold(),
                f"{group} status checks omitted the aggregate readiness finding")
        for finding in findings:
            observed_at = observed(finding)
            require(finding["source"]["kind"] == "readiness"
                    and 0 <= finding["start"] < finding["end"]
                    and finding["end"] - finding["start"] == len(finding["text"])
                    and observed_at.tzinfo is not None
                    and observed_at > prior_observation,
                    f"{group} status checks lacked fresh, valid readiness evidence")
        return findings

    gateway = settled_turn(
        request, context, "gateway", "What does this mean?",
        source={"kind": "readiness", "id": "gateway"}, mode="grounded",
    )
    gateway_finding = cited_finding(gateway, "readiness", "gateway")
    require(gateway_finding["text"]
            == "Gateway is ready. An authenticated gateway request succeeded.",
            "gateway answer did not report the observed ready state")

    unresolved = check_status_group(
        "what are the current unresolved status checks about?", "unresolved", observed(gateway_finding),
    )
    catalog_finding = next((finding for finding in unresolved[1:]
                            if finding["source"]["id"] == "actor_catalog"), None)
    require("Actor catalog" in unresolved[0]["text"] and catalog_finding is not None
            and catalog_finding["text"]
            == "Actor catalog is not ready. No completed seed was verified, and existing records may remain.",
            "unresolved status checks did not preserve the actual Actor catalog seed status")
    passed = check_status_group(
        "what are tthe green status check?", "passed", max(map(observed, unresolved)),
    )
    passed_summary = passed[0]["text"].casefold()
    require("gateway" in passed_summary and "registration" in passed_summary,
            "passed status checks omitted the ready gateway or verified registration")
    require("actor catalog" not in passed_summary,
            "passed status checks included the unseeded Actor catalog")

    active_jobs = running_jobs()
    jobs = ask("jobs", "any jobs running?")
    cited_running_jobs(jobs, active_jobs)
    questions = (
        ("cpu", "What is the current CPU utilization?", "CPU usage"),
        ("memory", "What is the current memory usage?", "memory usage"),
        ("disk", "What is the current disk usage?", "storage volume usage"),
    )
    readings = {}
    for source_id, metric_question, label in questions:
        metric = ask(source_id, metric_question)
        readings[source_id] = cited_metric(metric, source_id, label)
    memory_again = ask("memory-follow-up", "How much memory consumption we are at?", mode="preferred")
    readings["memory_follow_up"] = cited_metric(memory_again, "memory", "memory usage")
    require(datetime.fromisoformat(readings["memory_follow_up"]["observed_at"])
            > datetime.fromisoformat(readings["memory"]["observed_at"]),
            "memory follow-up reused the previous observation timestamp")
    greeting = ask("greeting", "hi", mode="dialogue")
    require(greeting.get("notice") == "greeting" and not greeting["findings"],
            "greeting did not receive a friendly reply without citations")
    worker, = model()
    dialogue_rss_bytes = embedded.read_process_rss_bytes(worker)
    preceding = readings["memory_follow_up"]
    for index, question in enumerate(("And now?", "Has that changed?"), start=1):
        turn = ask(f"conversational-follow-up-{index}", question, mode="preferred")
        measured = cited_metric(turn, "memory", "memory usage", previous=preceding)
        require(turn["question"] == question and observed(measured) > observed(preceding),
                "a conversational follow-up changed the displayed prompt or reused a metric sample")
        preceding = measured
    resources = ask("resources", "What is our resource usage?", mode="preferred")
    require(resources["state"] == "answered"
            and {part["source"]["id"] for part in resources["findings"]} == {"cpu", "memory", "disk"}
            and len(resources["findings"]) == 3
            and all(part["source"]["kind"] == "host_metric" and part.get("measurement")
                    for part in resources["findings"]),
            "resource summary did not cite exact CPU, memory and disk observations")
    help_turn = ask("help", "What can you help with?", mode="dialogue")
    require(help_turn.get("conversation") == "help" and not help_turn["findings"],
            "the assistant did not explain how to start a task")
    unsupported = ask("general-context", "What is the capital of Japan?")
    require(unsupported["state"] == "insufficient_context" and not unsupported["findings"]
            and unsupported.get("notice") == "out_of_scope"
            and unsupported.get("conversation") == "context_needed",
            "the assistant did not ask for grounding context")

    history = request("history")
    require(1 < len(history) <= 16 and history[0] == unsupported and jobs in history
            and all(turn["state"] in TERMINAL and not turn.get("provisional_reply") for turn in history),
            "Embedded history did not retain the current and earlier settled exchanges")

    require(model() == active, "Ordinary turns replaced or unloaded the resident model")
    started = time.monotonic()
    stop_model(request, registration, model, current)
    require(request("latest") is None, "Off retained latest question and answer")
    require(request("history") == [], "Off retained earlier questions or answers")
    query = {"turn_id": unsupported["turn_id"], "after": 0, "wait_seconds": 0}
    request("events", query=query, expected=404)
    return {"model_warm_ms": ready_ms, "admission_ms": admission_ms, "first_cited_finding_ms": first_finding_ms,
            "completion_ms": completed_ms, "unload_ms": round((time.monotonic() - started) * 1000, 1),
            "preferred_model_fallback_count": preferred_model_fallback_count,
            "model_pid_count": len(active), "warm_model_rss_bytes": worker_rss_bytes,
            "model_context": model_context,
            "fresh_observation": True, "terminal_events": terminal_events, "terminal_state": snapshot["state"],
            "grounded_readiness": True, "unresolved_status_checks": True, "passed_status_checks": True,
            "live_metrics": True, "unsupported_context": True,
            "resource_snapshot": True, "greeting": True, "conversational_follow_ups": True,
            "generated_dialogue": True, "startup_stages": sorted(stages),
            "automatic_model_target": current["model_target"], "worker_reused": True,
            "local_dialogue_rss_bytes": dialogue_rss_bytes,
            "frozen_samples": readings}


def embedded_tree(parent, *, worker_path=None):
    rows = subprocess.check_output(["ps", "-ww", "-axo", "pid=,ppid=,args="], text=True, timeout=10)
    members = {parent}
    processes = []
    for row in rows.splitlines():
        pid, ppid, command = row.split(maxsplit=2)
        program = command.split(" --model-dir ", 1)[0]
        processes.append((int(pid), int(ppid), Path(program)))
    while True:
        seen = members | {pid for pid, ppid, _ in processes if ppid in members}
        if seen == members:
            break
        members = seen
    children = [pid for pid, _, program in processes if pid in members and (
        program == worker_path if worker_path is not None else program.name == Path(pack.WORKER).name
    )]
    for pid in {parent, *children}:
        embedded.check_zero_bound_sockets(pid)
    return children


def sample_smaps_rollup(proc, phase_usage):
    try:
        rollup = (proc / "smaps_rollup").read_text()
    except OSError:
        return
    sample = {
        f"{name.lower()}_bytes": int(kib) * 1024
        for name, kib in re.findall(r"^([A-Za-z_]+):[ \t]+(\d+) kB$", rollup, re.M)
        if name in SMAPS_ROLLUP_FIELDS
    }
    previous = phase_usage.get("smaps_rollup_peak_sample", {})
    if sample.get("rss_bytes", 0) > previous.get("rss_bytes", 0):
        phase_usage["smaps_rollup_peak_sample"] = {**sample, "stage_sample": phase_usage["samples"]}


def retain_timing(aggregate, elapsed):
    aggregate["count"] = aggregate.get("count", 0) + 1
    aggregate["total"] = aggregate.get("total", 0) + elapsed
    aggregate["max"] = max(aggregate.get("max", 0), elapsed)


def sanitized_shutdown_ack(raw):
    if not isinstance(raw, dict):
        return None
    ack = {}
    if type(raw.get("checkpointed")) is bool:
        ack["checkpointed"] = raw["checkpointed"]
    leases = raw.get("relinquished_leases")
    if type(leases) is int and 0 <= leases < 2**64:
        ack["relinquished_leases"] = leases
    return ack or None


def post_ack_wait_summary(proc):
    def read(path, limit):
        try:
            with path.open(encoding="ascii", errors="replace") as stream:
                return stream.read(limit)
        except OSError:
            return ""

    def wait_channel(path):
        symbol = read(path, 64).strip()
        if symbol in SHUTDOWN_WAIT_CHANNELS:
            return symbol
        return "unspecified" if symbol in {"", "0"} else "other"

    status = read(proc / "status", 8192)
    state = re.search(r"^State:[ \t]+([RSDZTtXxIKWP])(?=[ \t]|$)", status, re.M)
    summary = {"main_wait_channel": wait_channel(proc / "wchan"), "task_wait_channels": {}}
    if state:
        summary["state"] = state[1]
    for name, key in (("Threads", "reported_threads"), ("VmRSS", "rss_kib")):
        value = re.search(rf"^{name}:[ \t]+([0-9]{{1,12}})(?: kB)?$", status, re.M)
        if value:
            summary[key] = int(value[1])
    try:
        with os.scandir(proc / "task") as tasks:
            entries = list(islice(tasks, SHUTDOWN_TASK_LIMIT + 1))
    except OSError:
        summary["task_snapshot_available"] = False
        return summary

    summary["task_snapshot_available"] = True
    summary["task_snapshot_truncated"] = len(entries) > SHUTDOWN_TASK_LIMIT
    counts = summary["task_wait_channels"]
    for entry in entries[:SHUTDOWN_TASK_LIMIT]:
        channel = wait_channel(Path(entry.path) / "wchan")
        counts[channel] = counts.get(channel, 0) + 1
    summary["sampled_tasks"] = sum(counts.values())
    return summary


class CatalogIpcSession(embedded.LocalIpcSession):
    def __init__(self, binary, state, evidence):
        self.evidence = evidence
        self.started = time.monotonic()
        self.index_batch_timings = {}
        self.index_pressure_transitions = {}
        self.index_merge_plans = {}
        self.seed_batch_timings = {}
        self.seed_phase_timings = {}
        self.shutdown_phases = {}
        self.rocksdb_shutdown_phases = {}
        super().__init__(binary, state, client_name="fleetworks-catalog-smoke",
                         log_filter="warn,surrealdb::core::kvs::rocksdb=info,"
                                    "argos_api_host::actor_index_worker=info,fleetworks_search_index::engine=info,"
                                    "argos_osint_core::storage::actor_catalog=debug,argos_api_host::setup_jobs=debug,"
                                    "argos_api_host::local_ipc::embedded=debug")

    def _record_index_batch(self, event):
        matches = INDEX_BATCH_FIELD.findall(event)
        fields = {name: int(value) for name, value in matches}
        if len(matches) != 4 or len(fields) != 4:
            return
        stage = self.evidence["stage"]
        timings = self.index_batch_timings.setdefault(stage, {"batches": 0, "mutations": 0})
        timings["batches"] += 1
        timings["mutations"] += fields["mutations"]
        for name in INDEX_BATCH_DURATIONS:
            elapsed = fields[name]
            aggregate = timings.setdefault(name, {"total": 0, "max": 0})
            aggregate["total"] += elapsed
            aggregate["max"] = max(aggregate["max"], elapsed)

    def _retain_event(self, stages, fields):
        stage = stages.setdefault(self.evidence["stage"], {"count": 0, "first": []})
        sample = {**fields, "elapsed_ms": round((time.monotonic() - self.started) * 1000)}
        stage["count"] += 1
        if len(stage["first"]) < CATALOG_EVENT_MILESTONES:
            stage["first"].append(sample)
        stage["latest"] = sample
        return stage

    def _record_index_event(self, text):
        if "embedded_actor_index_" not in text:
            return False
        event = ANSI.sub("", text)
        code = INDEX_EVENT.search(event)
        if code is None:
            return False
        kind = code[1]
        if kind == "batch_elapsed":
            self._record_index_batch(event)
        elif kind == "pressure_transition":
            paused = INDEX_PRESSURE_PAUSED.findall(event)
            reasons = INDEX_PRESSURE_REASON.findall(event)
            bytes_read = INDEX_PRESSURE_FIELD.findall(event)
            fields = {name: int(value) for name, value in bytes_read}
            if (len(paused) == 1 and len(reasons) == 1 and reasons[0] in INDEX_PRESSURE_REASONS
                    and len(bytes_read) == len(fields) == 2 and all(value < 2**64 for value in fields.values())):
                self._retain_event(self.index_pressure_transitions, {
                    "paused": paused[0] == "true", "reason": reasons[0], **fields,
                })
        else:
            matches = INDEX_MERGE_FIELD.findall(event)
            fields = {name: int(value) for name, value in matches}
            if len(matches) == len(fields) == 2:
                stage = self._retain_event(self.index_merge_plans, fields)
                for name, value in fields.items():
                    total = f"total_{name}"
                    stage[total] = stage.get(total, 0) + value
        return True

    def _record_seed_or_shutdown_event(self, text):
        if "embedded_actor_seed_" not in text and "embedded_backend_shutdown_phase" not in text:
            return False
        event = ANSI.sub("", text)
        code = SEED_EVENT.search(event)
        if code is None:
            return False
        if code[1] == "actor_seed_batch_elapsed":
            matches = SEED_BATCH_FIELD.findall(event)
            fields = {name: int(value) for name, value in matches}
            if (len(matches) == len(fields) == 10 and all(value < 2**64 for value in fields.values())
                    and 0 < fields["rows"] <= 128 and fields["rows"] <= fields["checkpoint"] <= CATALOG_ROWS):
                timings = self.seed_batch_timings
                timings["latest_checkpoint"] = fields.pop("checkpoint")
                for name, value in fields.items():
                    retain_timing(timings.setdefault(name, {}), value)
            return True
        phases = CATALOG_PHASE.findall(event)
        if len(phases) != 1:
            return True
        phase, = phases
        if code[1] == "actor_seed_phase_elapsed":
            elapsed = SEED_PHASE_ELAPSED.findall(event)
            if phase in SEED_PHASES and len(elapsed) == 1 and int(elapsed[0]) < 2**64:
                retain_timing(self.seed_phase_timings.setdefault(phase, {}), int(elapsed[0]))
        elif phase in SHUTDOWN_PHASES:
            self._retain_event(self.shutdown_phases, {"phase": phase})
        return True

    def _record_rocksdb_shutdown_event(self, text):
        if "in-flight compactions to drain" not in text and "Cancelling background work" not in text:
            return False
        match = ROCKSDB_SHUTDOWN_INFO.search(ANSI.sub("", text))
        if match is None:
            return False
        if match[1] is None:
            fields = {"phase": "cancelling_background_work"}
        else:
            fields = {"phase": "compaction_drain", "timeout_seconds": int(match[1])}
        self._retain_event(self.rocksdb_shutdown_phases, fields)
        return True

    def _drain_stderr(self):
        captured = 0
        while True:
            try:
                line = self.proc.stderr.readline()
            except (OSError, ValueError):
                return
            if not line:
                return
            text = line.decode("utf-8", errors="replace").rstrip("\n")
            if (self._record_index_event(text) or self._record_seed_or_shutdown_event(text)
                    or self._record_rocksdb_shutdown_event(text)):
                continue
            captured += len(text)
            if captured <= embedded.MAX_STDERR_TAIL_BYTES * 8:
                self._stderr_tail.append(text[:512])

    def close_unconditionally(self):
        if self.proc.poll() is not None:
            self._stderr_thread.join(timeout=2)
        super().close_unconditionally()
        self._stderr_thread.join(timeout=2)


def sample_catalog_resources(session, state, evidence, stop):
    proc = Path(f"/proc/{session.proc.pid}")
    while session.proc.poll() is None:
        phase = evidence["stage"]
        try:
            status = (proc / "status").read_text()
            disk_free = shutil.disk_usage(state).free
        except OSError:
            break
        if "embedded_actor_index_seed_deferred" not in evidence and re.search(
            r"\bembedded_actor_index_seed_deferred\b", session.stderr_excerpt()
        ):
            evidence["embedded_actor_index_seed_deferred"] = {"observed_stage": phase}
        if phase == "actor_seed" and evidence.get("actor_seed", {}).get("recovery_observed"):
            phase = "actor_seed_recovery"
        usage = {
            name: int(kib) * 1024
            for name, kib in re.findall(r"^(VmRSS|VmHWM|RssAnon|RssFile):\s+(\d+) kB$", status, re.M)
        }
        evidence.setdefault("baseline_disk_free_bytes", disk_free)
        if "VmRSS" in usage:
            evidence.setdefault("baseline_rss_bytes", usage["VmRSS"])
            evidence["peak_sampled_rss_bytes"] = max(evidence.get("peak_sampled_rss_bytes", 0), usage["VmRSS"])
        evidence["min_disk_free_bytes"] = min(evidence.get("min_disk_free_bytes", disk_free), disk_free)
        phase_usage = evidence["resources_by_stage"].setdefault(phase, {"samples": 0})
        phase_usage["samples"] += 1
        phase_usage["min_disk_free_bytes"] = min(phase_usage.get("min_disk_free_bytes", disk_free), disk_free)
        if "VmRSS" in usage:
            rss = usage["VmRSS"]
            phase_usage["min_sampled_rss_bytes"] = min(phase_usage.get("min_sampled_rss_bytes", rss), rss)
            phase_usage["last_sampled_rss_bytes"] = rss
        for field, name in (("peak_sampled_rss_bytes", "VmRSS"), ("peak_sampled_anon_bytes", "RssAnon"),
                            ("peak_sampled_file_bytes", "RssFile"), ("kernel_rss_high_water_bytes", "VmHWM")):
            if name in usage:
                phase_usage[field] = max(phase_usage.get(field, 0), usage[name])
        # Read the kernel aggregate every ten samples. Retain only numeric fields from one sample.
        if phase_usage["samples"] % 10 == 1:
            sample_smaps_rollup(proc, phase_usage)
        if stop.wait(1):
            break


def persisted_rocksdb_options(state):
    files = list((state / "surreal").rglob("OPTIONS-*"))
    require(files, "Embedded RocksDB did not persist an OPTIONS file")
    options = {}
    source = max(files, key=lambda path: path.name).read_text()
    for key, value in re.findall(r"^[ \t]*([a-z_]+)=(\d+)[ \t]*$", source, re.M):
        if key in ROCKSDB_OPTIONS:
            require(key not in options or options[key] == int(value), "RocksDB options disagree across column families")
            options[key] = int(value)
    return options


def staged_files(directory):
    files = {}
    for path in directory.iterdir():
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            # Publication can unlink the old name during a sample.
            continue
        if stat.S_ISREG(metadata.st_mode):
            files[path.name] = ((metadata.st_dev, metadata.st_ino), metadata.st_size)
    return files


def finish_bundled_stage(call, upload_id, staging, size, evidence):
    before = staged_files(staging)
    require(len(before) == 1, "bundled upload did not stage one private file")
    name, (inode, received_bytes) = next(iter(before.items()))
    # The last acknowledged chunk can still be buffered before Finish flushes it.
    require(name.endswith(".part") and 0 <= received_bytes <= size,
            "private upload file differs from its size pin")

    done = threading.Event()
    peak = received_bytes
    samples = 0
    sampling_error = None

    def sample():
        nonlocal peak, samples, sampling_error
        try:
            while not done.wait(0.005):
                sizes = dict(staged_files(staging).values())
                peak = max(peak, sum(sizes.values()))
                samples += 1
        except OSError as error:
            sampling_error = error

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    started = time.monotonic()
    try:
        upload = call("dataset_upload_finish", {"upload_id": upload_id})
    finally:
        elapsed_ms = round((time.monotonic() - started) * 1000)
        done.set()
        sampler.join()
        evidence.update(peak_sampled_distinct_inode_bytes=peak, publication_samples=samples,
                        validation_publish_ms=elapsed_ms)
    if sampling_error is not None:
        raise sampling_error
    published = f'{upload["dataset"]["id"]}.csv'
    require(staged_files(staging) == {published: (inode, size)},
            "bundled publication did not preserve the private upload inode")
    peak = max(peak, size)
    evidence["peak_sampled_distinct_inode_bytes"] = peak
    require(peak == size, "bundled validation staged more than one copy of the dataset")
    evidence["same_inode_published"] = True
    return upload


def stage_bundled_csv(call, transfer, directory, kind, pin, rows, *, staging, evidence):
    path = directory / pin["filename"]
    require(path.stat().st_size == pin["size_bytes"], "bundled dataset byte count changed")
    ticket = call("dataset_upload_start", {
        "kind": kind, "sha256": pin["sha256"], "size_bytes": pin["size_bytes"],
    })
    upload_id = ticket["upload_path"].rstrip("/").rsplit("/", 1)[-1]
    started = time.monotonic()
    with path.open("rb") as file:
        offset, control_ms = transfer(file, upload_id)
    require(offset == pin["size_bytes"] and control_ms is not None,
            "bundled binary upload or concurrent control request was incomplete")
    evidence.update(transfer_bytes=offset, transfer_ms=round((time.monotonic() - started) * 1000),
                    concurrent_readiness_ms=control_ms)
    upload = finish_bundled_stage(call, upload_id, staging, pin["size_bytes"], evidence)
    dataset = upload["dataset"]
    require(upload["row_count"] == rows and dataset["size_bytes"] == pin["size_bytes"]
            and dataset["sha256"] == pin["sha256"]
            and pack.sha256(staging / f'{dataset["id"]}.csv') == pin["sha256"],
            "backend validation did not match the bundled digest and row count")
    evidence.update(sha256=dataset["sha256"], validated_rows=upload["row_count"])
    return dataset


def exercise_bundled_catalog(session, state, app, evidence, *, stage_only):
    phase_started = time.monotonic()

    def next_stage(stage):
        nonlocal phase_started
        now = time.monotonic()
        evidence["elapsed_by_stage_ms"][evidence["stage"]] = round((now - phase_started) * 1000)
        evidence["stage"] = stage
        phase_started = now

    def shutdown():
        next_stage("shutdown")
        embedded.check_zero_bound_sockets(session.proc.pid)
        ack, exit_code = session.shutdown("bundled_catalog_smoke_complete")
        evidence["shutdown_ack"] = sanitized_shutdown_ack(ack)
        evidence["shutdown_exit_code"] = exit_code
        require(evidence["shutdown_ack"] is not None and evidence["shutdown_ack"].get("checkpointed") is True,
                "bundled catalog companion did not acknowledge a checkpointed shutdown")
        require(exit_code == 0, "bundled catalog companion did not shut down cleanly")
        next_stage("complete")

    require(session.hello()["compatible"], "packaged companion rejected the fixture protocol")
    evidence["transport_attestation"] = session.transport_attestation
    embedded.check_zero_bound_sockets(session.proc.pid)
    evidence["rocksdb_options"] = persisted_rocksdb_options(state)

    def block_cache_capacities():
        log = ANSI.sub("", session.stderr_excerpt())
        return {int(capacity) for capacity in re.findall(r"\bMemory manager: block cache size: (\d+)B\b", log)}

    capacities = until(block_cache_capacities, bool, "RocksDB startup omitted its block cache capacity", timeout=5)
    evidence["rocksdb_block_cache_bytes"] = sorted(capacities)
    require(evidence["rocksdb_options"] == ROCKSDB_OPTIONS, "persisted RocksDB options differ from the Embedded profile")
    require(capacities == {ROCKSDB_BLOCK_CACHE_BYTES}, "RocksDB startup did not report the bounded block cache")
    ensured, _ = session.request_json({"op": "registration_ensure", "args": {
        "request_id": "bundled-catalog-registration", "workstation_ip": None,
        "installation_marker": secrets.token_hex(32),
    }})
    response, _ = session.request_json({"op": "registration_context", "args": {
        "registration_id": ensured["registration_id"], "expected_revision": ensured["revision"],
    }})
    registration = response["context"]
    mutation_context = {**authorize(registration), "expected_configuration_revision": 0}

    def call(operation, args=None, *, allow_non_json=False, diagnostic=None):
        request = {"op": operation}
        if args is not None:
            request["args"] = args
        outcome, _ = session.request(request, registration=registration)
        data = outcome["data"]
        if outcome["outcome"] != "ok":
            if diagnostic is not None:
                evidence[diagnostic] = data.get("status")
                return None
            evidence["rpc_error"] = {"operation": operation, "status": data.get("status")}
            if (operation == "reindex_actors" and data.get("status") == 422
                    and data.get("message", "").startswith("Actor index status is still checking or catching up.")):
                evidence["actor_index_admission_waits"] = evidence.get("actor_index_admission_waits", 0) + 1
                return None
            raise ValueError("bundled catalog IPC operation failed")
        require(allow_non_json or data["payload"] in ("json", "dataset_upload"), "unexpected catalog IPC payload")
        return data.get("value")

    def check(check_id):
        checks = call("workstation_readiness")["checks"]
        return next(item for item in checks if item["id"] == check_id)

    def jobs():
        return call("jobs_list", {"sort": "newest"})["rows"]

    def seed_checkpoint(job_id, dataset, *, terminal):
        diagnostic = None if terminal else "checkpoint_rpc_status"
        detail = call("job_detail", {"id": job_id}, diagnostic=diagnostic)
        if detail is None or not detail["private_details_available"]:
            return None
        committed = detail["maintenance_result"]
        if (not committed or committed["operation"] != "actor_seed"
                or committed.get("dataset_sha256") != dataset["sha256"]):
            return None
        processed, total = committed.get("processed"), committed.get("total")
        if type(processed) is not int or total != CATALOG_ROWS or not 0 <= processed <= total:
            return None
        evidence.pop("checkpoint_rpc_status", None)
        return {
            "committed_rows": processed, "committed_total": total,
            "database_seeded": committed["database_seeded"],
            "checkpoint_observed_elapsed_ms": round((time.monotonic() - phase_started) * 1000),
        }

    def await_job(receipt, operation):
        job_id = receipt["job_id"]
        require(job_id and not receipt["already_completed"], "fresh catalog job was not durably accepted")
        checkpoint = {}
        recovery_observed = False
        next_checkpoint = 0
        max_reported_rows = 0
        while True:
            row = next((row for row in jobs() if row["id"] == job_id and row["operation"] == operation), None)
            if row is not None:
                state = row["state"]
                # Never retain job IDs, arbitrary error messages, titles or row payloads.
                observed = {
                    "state": state if state in TERMINAL_JOB_STATES | ACTIVE_JOB_STATES else "other",
                    "reported_rows": row.get("progress_done"), "expected_rows": row.get("progress_total"),
                }
                terminal = state in TERMINAL_JOB_STATES
                if row.get("error_summary") in SAFE_CATALOG_FAILURES:
                    observed["reason"] = row["error_summary"]
                    reasons = evidence.setdefault("job_reasons_seen", [])
                    if observed["reason"] not in reasons:
                        reasons.append(observed["reason"])
                if operation == "actor_seed":
                    max_reported_rows = max(max_reported_rows, observed["reported_rows"] or 0)
                    recovery_observed = (
                        recovery_observed or bool(observed.get("reason"))
                        or state in {"retry_scheduled", "waiting", "reconciling"}
                    )
                    observed.update(checkpoint, max_reported_rows=max_reported_rows, recovery_observed=recovery_observed)
                    progress = evidence.setdefault("actor_seed_progress", {})
                    samples = progress.setdefault("during_recovery" if recovery_observed else "initial", [])
                    rows = observed["reported_rows"]
                    if (type(rows) is int and len(samples) < CATALOG_PROGRESS_MILESTONES
                            and (not samples or rows > samples[-1]["reported_rows"])):
                        samples.append({
                            "reported_rows": rows, "elapsed_ms": round((time.monotonic() - phase_started) * 1000),
                        })
                evidence[operation] = observed
                now = time.monotonic()
                if operation == "actor_seed" and (terminal or (recovery_observed and now >= next_checkpoint)):
                    latest = seed_checkpoint(job_id, receipt["dataset"], terminal=terminal)
                    next_checkpoint = now + CHECKPOINT_POLL_SECONDS
                    if latest is not None:
                        checkpoint = latest
                        observed.update(checkpoint)
                        samples = evidence["actor_seed_progress"].setdefault("committed_checkpoints", [])
                        rows = checkpoint["committed_rows"]
                        if (len(samples) < CATALOG_PROGRESS_MILESTONES
                                and (not samples or rows != samples[-1]["committed_rows"])):
                            samples.append({
                                "committed_rows": rows, "elapsed_ms": checkpoint["checkpoint_observed_elapsed_ms"],
                            })
                if terminal:
                    return observed
            time.sleep(2)

    pins = json.loads((ROOT / "datasets/native-defaults.json").read_text())["datasets"]
    directory = app / "share/fleetworks/resources/datasets"
    require(check("actor_catalog")["state"] != "ready", "fresh catalog reported ready without a seed")
    next_stage("actor_csv_validation")
    evidence["actor_staging"] = {}
    def transfer(file, upload_id):
        return session.upload_dataset_bytes(file, upload_id, registration)

    dataset = stage_bundled_csv(call, transfer, directory, "actor_catalog", pins["actor_catalog"], CATALOG_ROWS,
                                staging=state / "staged-datasets", evidence=evidence["actor_staging"])
    evidence["actor_validated_rows"] = CATALOG_ROWS
    require(check("actor_catalog")["state"] != "ready", "staged catalog reported ready without a seed")
    if stage_only:
        shutdown()
        return

    next_stage("actor_seed")
    receipt = call("seed_actors", {
        "request_id": "bundled-catalog-seed", "context": mutation_context, "dataset": dataset,
    })
    seed = await_job(receipt, "actor_seed")
    evidence["actor_catalog_ready"] = check("actor_catalog")["state"] == "ready"
    require(seed["state"] == "completed" and seed.get("database_seeded") is True
            and seed.get("committed_rows") == seed.get("committed_total") == CATALOG_ROWS,
            "full bundled actor seed did not complete")
    if not evidence["actor_catalog_ready"]:
        until(lambda: check("actor_catalog"), lambda value: value["state"] == "ready",
              "completed actor catalog was not ready", timeout=30)
        evidence["actor_catalog_ready"] = True

    # The final catalog row proves that the serving index covers the tail of the import.
    source_id = "ucdp:999"
    actor_id = "catalog-actor-" + hashlib.sha256(source_id.encode()).hexdigest()[:24]
    query = {"query": "Chakosi", "mode": "discovery", "limit": 10}

    def retain_index_observation(kind, snapshot):
        progress = evidence.setdefault("actor_index_progress", {})
        series = progress.setdefault(kind, {"milestones": []})
        entry = {
            "stage": evidence["stage"], "elapsed_ms": round((time.monotonic() - phase_started) * 1000),
            "observation": snapshot,
        }
        previous = series.get("latest", {}).get("observation")
        if snapshot != previous and len(series["milestones"]) < CATALOG_PROGRESS_MILESTONES:
            series["milestones"].append(entry)
        series["latest"] = entry
        return progress

    def retain_index_checkpoint(response):
        index = response["index"]
        snapshot = {"state": index["state"] if index["state"] in SAFE_INDEX_STATES else "other"}
        for key in ("committed_checkpoint", "visible_checkpoint", "pending_work"):
            value = index.get(key)
            if type(value) is int and value >= 0:
                snapshot[key] = value
        progress = retain_index_observation("checkpoints", snapshot)
        visible = snapshot.get("visible_checkpoint")
        if visible is not None:
            progress["max_visible_checkpoint"] = max(progress.get("max_visible_checkpoint", 0), visible)

    next_stage("actor_index_catch_up")
    next_index_probe = 0

    def serving_index_ready():
        nonlocal next_index_probe
        observation = check("actor_search")
        state = observation["state"]
        reason = observation.get("reason_code")
        snapshot = {"state": state if state in SAFE_INDEX_STATES else "other"}
        if reason in SAFE_INDEX_REASONS:
            evidence["actor_search_readiness"] = reason
            snapshot["reason"] = reason
        retain_index_observation("readiness", snapshot)
        ready = state == "ready" and reason == "index_reader_visible"
        now = time.monotonic()
        if not ready and now >= next_index_probe:
            # The aggregate readiness response omits index counters. Sample the public search view.
            next_index_probe = now + INDEX_PROGRESS_POLL_SECONDS
            response = call("actors_search", {**query, "limit": 1}, diagnostic="actor_index_probe_rpc_status")
            if response is not None:
                evidence.pop("actor_index_probe_rpc_status", None)
                retain_index_checkpoint(response)
        return ready

    # The Embedded source drains automatically. The shared catalog watchdog still bounds this wait.
    until(serving_index_ready, bool, "initial actor index outbox did not become reader-visible",
          timeout=CATALOG_TIMEOUT_SECONDS, interval=2)

    next_stage("actor_index_admission")
    receipt = until(lambda: call("reindex_actors", {
        "request_id": "bundled-catalog-reindex", "context": mutation_context,
    }), bool, "packaged actor index could not be accepted", timeout=120)
    evidence.pop("rpc_error", None)
    require(receipt["already_completed"] and not receipt["job_id"]
            and not any(row["operation"] == "actor_reindex" for row in jobs()),
            "reader-visible initial index queued a redundant rebuild")
    evidence["actor_reindex"] = {"state": "already_completed"}

    next_stage("actor_search")

    def exact_actor_visible(response):
        index = response["index"]
        return index["state"] == "ready" and any(
            hit["actor_id"] == actor_id and hit["label"] == query["query"]
            and hit["index_generation"] == index["generation"] for hit in response["rows"]
        )

    search = until(lambda: call("actors_search", query), exact_actor_visible,
                   "bundled catalog tail actor was not reader-visible", timeout=180)
    index = search["index"]
    require(index["generation"] and index["visible_checkpoint"], "ready index lacks a visible serving generation")
    retain_index_checkpoint(search)
    evidence["actor_search"] = {"source_id": source_id, "visible_checkpoint": index["visible_checkpoint"]}

    next_stage("verify_retired_classifier_maintenance")
    readiness = call("workstation_readiness")
    require(not any(action["id"] == "train_classifier" and action["allowed"]
                    for probe in readiness["checks"] for action in probe.get("actions", [])),
            "the companion advertised a retired Natural training operation")
    require(not any(row["operation"] == "classifier_train" for row in jobs()),
            "a retired Natural training operation was accepted")

    shutdown()


def packaged_catalog_smoke(binary, app, state, *, stage_only):
    evidence = {"status": "failed", "stage": "startup", "resources_by_stage": {}, "elapsed_by_stage_ms": {}}
    evidence["scope"] = "stage_only" if stage_only else "full_catalog"
    require([name for _, name in socket.if_nameindex()] == ["lo"], "bundled catalog smoke needs an offline namespace")
    state.mkdir(mode=0o700)
    started = time.monotonic()
    stop = threading.Event()
    session = CatalogIpcSession(binary, state, evidence)

    def expire():
        if session.proc.poll() is None:
            evidence["timed_out"] = True
            session.proc.kill()

    timeout = threading.Timer(CATALOG_STAGE_TIMEOUT_SECONDS if stage_only else CATALOG_TIMEOUT_SECONDS, expire)
    timeout.daemon = True
    timeout.start()
    sampler = threading.Thread(target=sample_catalog_resources, args=(session, state, evidence, stop), daemon=True)
    sampler.start()
    failure_type = None
    try:
        exercise_bundled_catalog(session, state, app, evidence, stage_only=stage_only)
        if not stage_only:
            require(evidence.get("peak_sampled_rss_bytes", 0) > 0
                    and evidence["resources_by_stage"].get("actor_seed", {}).get("samples", 0) > 0,
                    "bundled catalog smoke did not collect seed resource observations")
        evidence["status"] = "passed"
    except (OSError, ValueError, KeyError, StopIteration, subprocess.SubprocessError) as error:
        failure_type = type(error).__name__
        if (evidence["stage"] == "shutdown" and isinstance(error, subprocess.TimeoutExpired)
                and sanitized_shutdown_ack(session.last_shutdown_ack) is not None and session.proc.poll() is None):
            evidence["shutdown_wait_snapshot"] = post_ack_wait_summary(Path(f"/proc/{session.proc.pid}"))
    finally:
        timeout.cancel()
        timeout.join(timeout=1)
        stop.set()
        sampler.join(timeout=5)
        session.close_unconditionally()
    if session.index_batch_timings:
        evidence["actor_index_batch_timings"] = session.index_batch_timings
    if session.index_pressure_transitions:
        evidence["actor_index_pressure_transitions"] = session.index_pressure_transitions
    if session.index_merge_plans:
        evidence["actor_index_merge_plans"] = session.index_merge_plans
    if session.seed_batch_timings:
        evidence["actor_seed_batch_timings"] = session.seed_batch_timings
    if session.seed_phase_timings:
        evidence["actor_seed_phase_timings"] = session.seed_phase_timings
    if session.shutdown_phases:
        evidence["backend_shutdown_phases"] = session.shutdown_phases
    if session.rocksdb_shutdown_phases:
        evidence["rocksdb_shutdown_phases"] = session.rocksdb_shutdown_phases
    last_ack = sanitized_shutdown_ack(getattr(session, "last_shutdown_ack", None))
    if last_ack is not None:
        evidence["shutdown_ack"] = last_ack
    if "min_disk_free_bytes" in evidence:
        evidence["peak_disk_headroom_drop_bytes"] = evidence["baseline_disk_free_bytes"] - evidence["min_disk_free_bytes"]
    evidence["elapsed_ms"] = round((time.monotonic() - started) * 1000)
    if evidence.get("timed_out"):
        evidence["status"] = "failed"
        evidence["failure_reason"] = "catalog_watchdog_deadline"
        failure_type = "TimeoutError"
    elif evidence["stage"] == "shutdown" and failure_type == "TimeoutExpired" and last_ack:
        evidence["failure_reason"] = "catalog_teardown_timeout"
    if evidence["status"] == "failed":
        evidence["failure_type"] = failure_type
        evidence["elapsed_by_stage_ms"][evidence["stage"]] = max(
            0, evidence["elapsed_ms"] - sum(evidence["elapsed_by_stage_ms"].values()),
        )
        print(f"Offline bundled catalog smoke failed during {evidence['stage']} ({failure_type})", file=sys.stderr)
    return evidence


def run_catalog(args):
    require(os.geteuid() == os.getuid() != 0 and os.getegid() == os.getgid(),
            "bundled catalog smoke requires the unprivileged runner account")
    require([name for _, name in socket.if_nameindex()] == ["lo"], "bundled catalog smoke needs an offline namespace")
    require(re.fullmatch(r"[0-9a-f]{40}", args.revision), "catalog package requires the exact source revision")
    app = args.app.resolve()
    binary = embedded.resolve_companion_binary(app=app)
    manifest = json.loads((app / "share/fleetworks/fleetworks-manifest.json").read_text())
    require(manifest["platform"] == "linux" and manifest["target"] == "x86_64-unknown-linux-gnu"
            and manifest["profile"] == "debug" and manifest["dirty"] is False
            and manifest["revision"] == args.revision and str(manifest["run_id"]) == args.run_id
            and str(manifest["run_attempt"]) == args.run_attempt, "catalog package provenance does not match this run")
    companion_sha = pack.sha256(binary)
    require(companion_sha == manifest["companion"]["sha256"], "catalog companion checksum differs from its manifest")
    with tempfile.TemporaryDirectory(prefix="fleetworks-bundled-catalog-") as temporary:
        result = packaged_catalog_smoke(binary, app, Path(temporary) / "state", stage_only=args.stage_only)
    return {
        "runtime": result.get("transport_attestation", {}).get("transport"),
        "target": manifest["target"], "source_sha": args.revision,
        "run_id": args.run_id, "run_attempt": args.run_attempt, "companion_sha256": companion_sha,
        "bundled_catalog": result,
    }


@contextmanager
def embedded_qa_session(binary, state, *, reject_model_target=False, qa_memory_mib=FIXTURE_MODEL_MEMORY_MIB,
                        isolated_root=None):
    session = embedded.LocalIpcSession(binary, state, client_name="fleetworks-qa-smoke",
                                       qa_memory_mib=qa_memory_mib, isolated_root=isolated_root)
    try:
        session.hello()
        ensured, _ = session.request_json({"op": "registration_ensure", "args": {
            "request_id": "qa-smoke-registration", "workstation_ip": None,
            "installation_marker": secrets.token_hex(32),
        }})
        response, _ = session.request_json({"op": "registration_context", "args": {
            "registration_id": ensured["registration_id"], "expected_revision": ensured["revision"],
        }})
        registration = response["context"]

        def request(path, payload=None, *, query=None, expected=200):
            operation = {"settings": "qa_settings", "turns": "qa_turn_submit", "latest": "qa_latest",
                         "history": "qa_history", "events": "qa_events", "cancel": "qa_cancel"}[path]
            if path == "settings" and payload is not None:
                operation = "qa_settings_update"
            request = {"op": operation}
            if payload is not None or query is not None:
                request["args"] = payload if payload is not None else query
            result, _ = session.request(request, registration=registration)
            data = result["data"]
            statuses = (expected,) if isinstance(expected, int) else expected
            if result["outcome"] == "ok":
                require(200 in statuses, f"IPC {operation} unexpectedly succeeded")
                return data["value"]
            reason = SAFE_REJECTION.get(data.get("message"), "unclassified")
            require(data["status"] in statuses,
                    f"IPC {operation} received {data['status']} ({reason})")
            return None

        yield session, registration, request
        if reject_model_target:
            # Unknown typed operation fields are rejected at the IPC frame boundary.
            session.send_request({"op": "qa_settings_update", "args": {
                "context": authorize(registration), "expected_revision": request("settings")["revision"],
                "enabled": True, "model_target": "flan-t5-base",
            }}, registration=registration)
            frame = embedded.read_frame(session.reader)
            while frame is not None and frame.get("frame") == "event":
                frame = embedded.read_frame(session.reader)
            require(frame is None and session.proc.wait(timeout=40) == 0,
                    "The companion did not fail closed on a caller-selected model")
        else:
            _, exit_code = session.shutdown()
            require(exit_code == 0, "companion shutdown failed after QA")
    except Exception:
        report_diagnostics(session.stderr_excerpt())
        raise
    finally:
        session.close_unconditionally()


def select_generator(request, context, preference, *, enable=False):
    settings = request("settings")
    update = {"context": context, "expected_revision": settings["revision"],
              "generator_preference": preference}
    if enable:
        update["enabled"] = True
    return request("settings", update)


def exercise_context_without_model(request, context):
    jobs = settled_turn(request, context, "running-jobs-list", "what jobs are running?")
    require(jobs["state"] == "answered"
            and any(finding["source"]["kind"] == "job_count" for finding in jobs["findings"]),
            "The running-jobs list did not cite the active workstation's job count")

    status = settled_turn(request, context, "status-check-shorthand", "status check?")
    require(status["state"] == "answered" and status["findings"],
            "The status shorthand did not return a readiness finding")
    finding = status["findings"][0]
    match = re.search(r"\b(\d+) failed, and ([1-9]\d*) unresolved\b", finding["text"])
    require(match is not None
            and finding["source"] == {"kind": "readiness", "id": "workstation_status_checks"},
            "The status fixture did not report any unresolved checks")
    followed_group = "failed" if match[1] != match[2] else "unresolved"
    follow_up = settled_turn(request, context, "status-check-referent",
                             f"what are the {match[2]} {followed_group} one?")
    require(follow_up["state"] == "answered" and follow_up.get("used_previous_exchange") is True,
            "The shorthand follow-up did not use the preceding status exchange")
    summary = follow_up["findings"][0]
    require(summary["source"] == finding["source"]
            and "unresolved" in summary["text"].casefold()
            and "actor catalog" in summary["text"].casefold(),
            "The follow-up did not identify the unresolved checks from current evidence")
    if followed_group == "failed":
        require(f"The earlier count of {match[2]} was for unresolved checks." in summary["text"],
                "The follow-up omitted the difference between failed and unresolved checks")
    recap = settled_turn(request, context, "recap-last-answer", "What was your last answer?")
    require(recap["state"] == "answered" and recap.get("used_previous_exchange") is True
            and "unresolved" in (recap.get("reply") or "").casefold(),
            "The rules path could not report the previous answer from retained context")


def verify_explicit_base_unavailable(request, registration, model, reason, *, enable=False):
    context = authorize(registration)
    updated = select_generator(request, context, "flan_t5_base", enable=enable)
    unavailable = until(lambda: request("settings"),
                        lambda settings: settings["generation"] == updated["generation"]
                        and settings["generator"]["phase"] == "unavailable",
                        "Explicit Base did not settle with its own startup failure", timeout=110)
    generator = unavailable["generator"]
    require(generator["preference"] == "flan_t5_base" and generator.get("effective") is None
            and generator["reason"] == reason and not model(),
            "Explicit Base was replaced by Small or reported the wrong availability reason")
    facts = settled_turn(request, context, f"explicit-base-{reason}", "What is the current memory usage?")
    cited_metric(facts, "memory", "memory usage")
    require(facts.get("model_target") is None and facts.get("model_generation") is None,
            "Direct facts acquired generator provenance while Explicit Base was unavailable")
    if reason == "insufficient_resources":
        exercise_context_without_model(request, context)
    stop_model(request, registration, model, unavailable)


def named_small_startup(args, binary, digests):
    with tempfile.TemporaryDirectory(prefix="fleetworks-embedded-qa-named-") as temporary:
        root = Path(temporary)
        state = root / "state"
        state.mkdir(mode=0o700)
        pack.install(argparse.Namespace(archive=args.archive, output=root / "pack", state_dir=state,
                                        target=digests["target"], archive_sha256=digests["archive_sha256"],
                                        manifest_sha256=digests["manifest_sha256"], chat_model="flan-t5-small"))
        require((state / "qa-model-packs/flan-t5-small.json").is_file()
                and not (state / "qa-pack.json").exists()
                and not (state / "qa-model-assets").exists(),
                "Named Small fallback requires only the named pack and an empty model cache")
        with embedded_qa_session(binary, state, qa_memory_mib=None) as (session, registration, request):
            context = authorize(registration)
            before = request("settings")
            enabled = request("settings", {"context": context, "expected_revision": before["revision"],
                                           "enabled": True})
            ready = ready_model(request, "flan-t5-small", updated=enabled)
            require("base_model_provision" in session.stderr_excerpt(),
                    "The offline fixture did not attempt Base with the available memory headroom")
            answer = settled_turn(request, context, "named-small-fallback", SUMMARY_QUESTION, mode="operational")
            require(all(finding["source"]["kind"] == "readiness" for finding in answer["findings"]),
                    "Named Small fallback did not preserve grounded readiness citations")
            require(ready["generator"]["preference"] == "automatic"
                    and ready["generator"]["effective"] == "flan-t5-small",
                    "Automatic did not describe its verified Small fallback")

            def model():
                return embedded_tree(session.proc.pid)

            auto_worker, = model()
            explicit = select_generator(request, context, "flan_t5_small")
            ready = ready_model(request, "flan-t5-small", updated=explicit)
            small_worker, = model()
            require(ready["generation"] == explicit["generation"]
                    and ready["generator"]["preference"] == "flan_t5_small"
                    and ready["generator"]["effective"] == "flan-t5-small"
                    and small_worker != auto_worker,
                    "Explicit Small did not warm its own generation on a Base-capable workstation")
            retained = request("events", query={"turn_id": answer["turn_id"], "after": answer["cursor"],
                                                "wait_seconds": 0})
            require(retained["turn"] == answer and not retained["events"],
                    "Changing generator cleared a completed exchange or duplicated its terminal event")
            explicit_answer = settled_turn(request, context, "explicit-small", SUMMARY_QUESTION, mode="operational")
            require(explicit_answer["model_target"] == "flan-t5-small"
                    and explicit_answer["model_generation"] == ready["generation"],
                    "Turn provenance did not identify the actual explicit Small worker and generation")

            for preference in ("julia1_q8", "vllm_d3_edge_q8", "rules_only"):
                updated = request("settings", {"context": context, "expected_revision": ready["revision"],
                                               "decision_preference": preference})
                ready = request("settings")
                decision = ready["decision"]
                require(decision["preference"] == preference and decision["effective"] == "rules_only"
                        and decision.get("reason") == (None if preference == "rules_only" else "awaiting_qualification")
                        and updated["generation"] == explicit_answer["model_generation"]
                        and ready["generator"]["effective"] == "flan-t5-small" and model() == [small_worker],
                        "Changing Decision displaced the generator or activated an unqualified scorer")

            still_warm = settled_turn(request, context, "decision-changes-kept-generator", "hi", mode="dialogue")
            require(still_warm["model_generation"] == explicit_answer["model_generation"]
                    and model() == [small_worker],
                    "The generator failed to complete a turn on its existing worker after Decision edits")

            verify_explicit_base_unavailable(request, registration, model, "model_provision_failed")
        return {"base_provision_attempted": True, "selected_model": "flan-t5-small",
                "grounded_answer": True, "explicit_small_worker": True,
                "decision_changes_kept_generator": True, "base_provision_error_preserved": True}


def run_embedded(args):
    # Run this program inside a network-denied environment for release verification.
    require(not any((Path.home() / path).exists() for path in (
        ".cache/torch", ".cache/huggingface", ".cache/.rustbert")), "use a fresh HOME for the offline smoke")
    binary = embedded.resolve_companion_binary(app=args.app)
    digests = json.loads(args.digests.read_text())
    linux = digests["target"] == "x86_64-unknown-linux-gnu"
    if linux:
        require({name for _, name in socket.if_nameindex()} == {"lo"},
                "Run the Linux QA smoke in a network namespace with only loopback")
    with tempfile.TemporaryDirectory(prefix="fleetworks-embedded-qa-") as temporary:
        root = Path(temporary)
        state = root / "state"
        state.mkdir(mode=0o700)
        pack.install(argparse.Namespace(archive=args.archive, output=root / "pack", state_dir=state,
                                        target=digests["target"], archive_sha256=digests["archive_sha256"],
                                        manifest_sha256=digests["manifest_sha256"]))
        with embedded_qa_session(binary, state, reject_model_target=True) as (session, registration, request):

            def model():
                return embedded_tree(session.proc.pid)

            def readiness():
                value, _ = session.request_json({"op": "workstation_readiness"}, registration=registration)
                return bool(value.get("deployment_id"))

            def running_jobs():
                value, _ = session.request_json({"op": "jobs_list", "args": {"sort": "newest"}}, registration=registration)
                require(value["dashboard_counts_scope"] == "unfiltered_top_level_workstation_activity",
                        "IPC jobs dashboard counts used the wrong scope")
                return value["counts"]["active"]

            for stage in ("downloading_pack", "verifying_pack"):
                with pack.pack_progress(state, stage):
                    settings = request("settings")
                    require(settings["phase"] == "off" and settings.get("startup_stage") == stage and not model(),
                            f"Embedded did not expose the advisory {stage} installer stage")
            require(not request("settings").get("startup_stage"), "Installer progress lingered after its producer exited")
            result = exercise(request, registration, model, readiness, running_jobs)
            if linux:
                verify_explicit_base_unavailable(request, registration, model, "insufficient_resources", enable=True)
                result["explicit_base_memory_admission"] = "insufficient_resources"
                result["contextual_status_without_model"] = True
            result["installer_stages"] = ["downloading_pack", "verifying_pack"]
            attestation = session.transport_attestation

    if linux:
        result["named_small_fallback"] = named_small_startup(args, binary, digests)

    with tempfile.TemporaryDirectory(prefix="fleetworks-embedded-qa-stop-") as temporary:
        root = Path(temporary)
        state = root / "state"
        state.mkdir(mode=0o700)
        worker, pointer = pack.install_gated_fixture(args.archive, root / "pack", args.gated_worker, digests)
        descriptor = os.open(state / "qa-pack.json", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "w") as file:
            json.dump(pointer, file)
        with embedded_qa_session(binary, state) as (session, registration, request):
            require(session.transport_attestation == attestation, "Stop fixture did not negotiate the same socket")
            def gated_model():
                return embedded_tree(session.proc.pid, worker_path=worker)

            cancellation = cancel_running_embedded_turn(request, registration, gated_model)
            stop_model(request, registration, gated_model, request("settings"))
            cancellation.update(cancel_running_embedded_turn(request, registration, gated_model, draft=True))
            stop_model(request, registration, gated_model, request("settings"))

    return {"runtime": attestation["transport"], "transport_attestation": attestation, "target": digests["target"],
            "source_sha": digests["source_sha"], "manual_model_target_rejected": True, **result, **cancellation}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="runtime", required=True)
    local = modes.add_parser("embedded")
    local.add_argument("--app", type=Path, required=True)
    local.add_argument("--archive", type=Path, required=True)
    local.add_argument("--digests", type=Path, required=True)
    local.add_argument("--gated-worker", type=Path, required=True)
    catalog = modes.add_parser("catalog", help="Run the offline bundled catalog smoke for the verified Linux PR package")
    catalog.add_argument("--app", type=Path, required=True)
    catalog.add_argument("--revision", required=True)
    catalog.add_argument("--run-id", required=True)
    catalog.add_argument("--run-attempt", required=True)
    catalog.add_argument("--stage-only", action="store_true", help="Verify single-copy bundled staging without seeding")
    args = parser.parse_args()
    if args.runtime == "embedded":
        os.environ["RUST_LOG"] = "warn,argos_api_host::inbox_qa_service=info"
        result = run_embedded(args)
    else:
        try:
            result = run_catalog(args)
        except (OSError, ValueError, KeyError) as error:
            result = {
                "source_sha": args.revision, "run_id": args.run_id, "run_attempt": args.run_attempt,
                "bundled_catalog": {"status": "failed", "stage": "package_preflight",
                                    "failure_type": type(error).__name__},
            }
    print(json.dumps(result, indent=2))
    return int(result.get("bundled_catalog", {}).get("status") == "failed")


if __name__ == "__main__":
    sys.exit(main())
